use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

use super::super::*;

#[test]
fn ensure_fresh_logs_repo_ref_and_refresh_plan() {
    let fixture = GitRepoFixture::new();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = CaptureSubscriber {
        captured: Arc::clone(&captured),
    };

    tracing::subscriber::with_default(subscriber, || {
        ensure_fresh(&fixture.knowledge_dir, &fixture.repo).expect("ensure fresh");
    });

    let logs = captured.lock().expect("lock captured logs").join("\n");
    assert!(logs.contains("orbit.knowledge.refresh"));
    assert!(logs.contains(&format!("repo_path={}", fixture.repo.display())));
    assert!(logs.contains("ref_name"));
    assert!(logs.contains("agent-main"));
    assert!(logs.contains("refresh_plan"));
    assert!(logs.contains("Rebuild"));
}

struct CaptureSubscriber {
    captured: Arc<Mutex<Vec<String>>>,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "orbit.knowledge.refresh" && *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        let mut visitor = FieldCapture::default();
        event.record(&mut visitor);
        let fields = visitor.fields.join(" ");
        self.captured
            .lock()
            .expect("lock captured events")
            .push(format!("{} {fields}", event.metadata().target()));
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct FieldCapture {
    fields: Vec<String>,
}

impl Visit for FieldCapture {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields.push(format!("{}={value:?}", field.name()));
    }
}

struct GitRepoFixture {
    _root: tempfile::TempDir,
    repo: PathBuf,
    knowledge_dir: PathBuf,
}

impl GitRepoFixture {
    fn new() -> Self {
        let root = tempdir().expect("create tempdir");
        let repo = root.path().join("repo");
        let knowledge_dir = root.path().join("knowledge");
        std::fs::create_dir_all(repo.join("src")).expect("create src dir");
        run_git(root.path(), &["init", repo.to_str().expect("repo path")]);
        run_git(&repo, &["config", "user.email", "test@example.com"]);
        run_git(&repo, &["config", "user.name", "Test User"]);
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"refresh_log_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write manifest");
        std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").expect("write lib");
        run_git(&repo, &["add", "Cargo.toml", "src/lib.rs"]);
        run_git(&repo, &["commit", "-m", "initial"]);
        run_git(&repo, &["branch", "-M", "agent-main"]);

        Self {
            _root: root,
            repo,
            knowledge_dir,
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
