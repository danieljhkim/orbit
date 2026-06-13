//! Worktree scanner and file-table diffing.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use fs2::FileExt;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use orbit_graph_extract::Extractor;
use orbit_graph_extract::languages;
use rusqlite::{Connection, params};

use crate::{GraphError, SyncMode};

const ORBITIGNORE_FILE_NAME: &str = ".orbitignore";

const DEFAULT_ORBITIGNORE_PATTERNS: &[&str] = &[
    ".orbit/",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    ".venv/",
    "venv/",
    "__pycache__/",
    "*.egg-info/",
];

const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "woff", "woff2", "ttf", "eot", "exe", "dll", "so", "dylib",
    "pdf", "zip", "tar", "gz", "lock",
];

/// Per-file classification produced by the scanner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Diff {
    /// Existing indexed files whose content remains current.
    pub(crate) unchanged: Vec<PathBuf>,
    /// Existing indexed files whose content changed or must be fully refreshed.
    pub(crate) modified: Vec<PathBuf>,
    /// Indexable files present on disk without a `files` row.
    pub(crate) new: Vec<PathBuf>,
    /// Files present in `files` but absent from the filtered worktree scan.
    pub(crate) deleted: Vec<PathBuf>,
}

#[cfg(test)]
pub(crate) fn scan_diff(
    db_path: &Path,
    worktree_root: &Path,
    mode: SyncMode,
) -> Result<Diff, GraphError> {
    Scanner::new(db_path, worktree_root)?.scan(mode, &Blake3Hasher)
}

pub(crate) fn scan_diff_with_lock_held(
    db_path: &Path,
    worktree_root: &Path,
    mode: SyncMode,
) -> Result<Diff, GraphError> {
    Scanner::new_with_lock(db_path, worktree_root, None).scan(mode, &Blake3Hasher)
}

struct Scanner {
    db_path: PathBuf,
    worktree_root: PathBuf,
    _lock: Option<DbLockGuard>,
    registry: ExtractorRegistry,
}

impl Scanner {
    #[cfg(test)]
    fn new(db_path: &Path, worktree_root: &Path) -> Result<Self, GraphError> {
        let lock = DbLockGuard::acquire(db_path)?;
        Ok(Self::new_with_lock(db_path, worktree_root, Some(lock)))
    }

    fn new_with_lock(db_path: &Path, worktree_root: &Path, lock: Option<DbLockGuard>) -> Self {
        Self {
            db_path: db_path.to_path_buf(),
            worktree_root: worktree_root.to_path_buf(),
            _lock: lock,
            registry: ExtractorRegistry::default(),
        }
    }

    fn scan(&self, mode: SyncMode, hasher: &dyn ContentHasher) -> Result<Diff, GraphError> {
        note_scan_started(self.worktree_root.as_path());

        let conn = Connection::open(self.db_path.as_path())
            .map_err(|source| GraphError::sqlite("open graph database for scan", source))?;
        let mut rows = load_file_rows(&conn)?;
        let orbitignore = OrbitIgnoreMatcher::load(self.worktree_root.as_path())?;
        let mut disk_files = Vec::new();
        walk_dir(
            self.worktree_root.as_path(),
            self.worktree_root.as_path(),
            &orbitignore,
            &self.registry,
            &mut disk_files,
        )?;
        disk_files.sort_by(|left, right| left.path.cmp(&right.path));

        let ignored = git_ignored_paths(self.worktree_root.as_path(), &disk_files);
        let mut diff = Diff::default();
        let mut seen = HashSet::new();

        for disk_file in disk_files {
            if ignored.contains(&disk_file.path) {
                continue;
            }
            seen.insert(disk_file.path.clone());

            let Some(existing) = rows.remove(&disk_file.path) else {
                let _ = hash_file(self.worktree_root.as_path(), &disk_file.path, hasher)?;
                diff.new.push(disk_file.path);
                continue;
            };

            if mode == SyncMode::Auto && existing.mtime_ns == disk_file.mtime_ns {
                diff.unchanged.push(disk_file.path);
                continue;
            }

            let content_hash = hash_file(self.worktree_root.as_path(), &disk_file.path, hasher)?;
            if mode == SyncMode::Auto && content_hash == existing.content_hash {
                touch_mtime(&conn, &disk_file.path, disk_file.mtime_ns)?;
                diff.unchanged.push(disk_file.path);
            } else {
                diff.modified.push(disk_file.path);
            }
        }

        diff.deleted
            .extend(rows.into_keys().filter(|path| !seen.contains(path)));
        sort_diff(&mut diff);
        Ok(diff)
    }
}

pub(crate) struct DbLockGuard {
    _file: File,
}

impl DbLockGuard {
    pub(crate) fn acquire(db_path: &Path) -> Result<Self, GraphError> {
        // L-0048: lock a sidecar so SQLite can still read the DB while the RAII guard is held.
        let lock_path = lock_path_for(db_path);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path.as_path())
            .map_err(|source| {
                GraphError::io("open graph database lock file", lock_path.as_path(), source)
            })?;
        file.lock_exclusive()
            .map_err(|source| GraphError::io("lock graph database", lock_path, source))?;
        Ok(Self { _file: file })
    }
}

fn lock_path_for(db_path: &Path) -> PathBuf {
    let mut lock_path = db_path.to_path_buf();
    let file_name = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "graph.db".to_string(), ToString::to_string);
    lock_path.set_file_name(format!("{file_name}.lock"));
    lock_path
}

struct ExtractorRegistry {
    extractors: Vec<Box<dyn Extractor>>,
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self {
            extractors: languages::extractors(),
        }
    }
}

impl ExtractorRegistry {
    fn language_for(&self, path: &Path) -> Option<&'static str> {
        self.extractors
            .iter()
            .find(|extractor| extractor.supports(path))
            .map(|extractor| extractor.lang())
    }
}

trait ContentHasher {
    fn hash(&self, path: &Path, bytes: &[u8]) -> Vec<u8>;
}

struct Blake3Hasher;

impl ContentHasher for Blake3Hasher {
    fn hash(&self, _path: &Path, bytes: &[u8]) -> Vec<u8> {
        blake3::hash(bytes).as_bytes().to_vec()
    }
}

#[derive(Debug)]
struct FileRow {
    content_hash: Vec<u8>,
    mtime_ns: i64,
}

#[derive(Debug)]
struct DiskFile {
    path: PathBuf,
    mtime_ns: i64,
}

struct OrbitIgnoreMatcher {
    gitignore: Gitignore,
}

impl OrbitIgnoreMatcher {
    fn load(repo_path: &Path) -> Result<Self, GraphError> {
        let mut builder = GitignoreBuilder::new(repo_path);
        add_default_orbitignore_patterns(&mut builder)?;
        let default_orbitignore = Self {
            gitignore: builder.build().map_err(|error| {
                GraphError::invalid_data("build default .orbitignore matcher", error.to_string())
            })?,
        };

        let mut orbitignore_files = Vec::new();
        collect_orbitignore_files(
            repo_path,
            repo_path,
            &default_orbitignore,
            &mut orbitignore_files,
        )?;
        orbitignore_files.sort_by(|left, right| {
            let left_rel = left.strip_prefix(repo_path).unwrap_or(left.as_path());
            let right_rel = right.strip_prefix(repo_path).unwrap_or(right.as_path());
            left_rel
                .components()
                .count()
                .cmp(&right_rel.components().count())
                .then_with(|| left_rel.cmp(right_rel))
        });

        for orbitignore in orbitignore_files {
            if let Some(error) = builder.add(&orbitignore) {
                return Err(GraphError::invalid_data(
                    "load .orbitignore",
                    format!("load {}: {error}", orbitignore.display()),
                ));
            }
        }

        let gitignore = builder.build().map_err(|error| {
            GraphError::invalid_data("build .orbitignore matcher", error.to_string())
        })?;
        Ok(Self { gitignore })
    }

    fn is_ignored(&self, rel_path: &Path, is_dir: bool) -> bool {
        self.gitignore
            .matched_path_or_any_parents(rel_path, is_dir)
            .is_ignore()
    }
}

fn add_default_orbitignore_patterns(builder: &mut GitignoreBuilder) -> Result<(), GraphError> {
    for pattern in DEFAULT_ORBITIGNORE_PATTERNS {
        builder.add_line(None, pattern).map_err(|error| {
            GraphError::invalid_data(
                "load default .orbitignore patterns",
                format!("invalid default .orbitignore pattern `{pattern}`: {error}"),
            )
        })?;
    }
    Ok(())
}

fn load_file_rows(conn: &Connection) -> Result<BTreeMap<PathBuf, FileRow>, GraphError> {
    let mut stmt = conn
        .prepare("SELECT path, content_hash, mtime_ns FROM files")
        .map_err(|source| GraphError::sqlite("prepare files scan query", source))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                FileRow {
                    content_hash: row.get(1)?,
                    mtime_ns: row.get(2)?,
                },
            ))
        })
        .map_err(|source| GraphError::sqlite("query files for scan", source))?;

    rows.collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|source| GraphError::sqlite("collect files scan rows", source))
}

fn touch_mtime(conn: &Connection, path: &Path, mtime_ns: i64) -> Result<(), GraphError> {
    conn.execute(
        "UPDATE files SET mtime_ns = ?1 WHERE path = ?2",
        params![mtime_ns, normalize_path(path)],
    )
    .map_err(|source| GraphError::sqlite("update unchanged file mtime", source))?;
    Ok(())
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    orbitignore: &OrbitIgnoreMatcher,
    registry: &ExtractorRegistry,
    out: &mut Vec<DiskFile>,
) -> Result<(), GraphError> {
    let entries =
        fs::read_dir(dir).map_err(|source| GraphError::io("scan directory", dir, source))?;

    for entry in entries {
        let entry = entry.map_err(|source| GraphError::io("read directory entry", dir, source))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|source| GraphError::io("read file type", path.as_path(), source))?;

        if file_type.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root)
                && orbitignore.is_ignored(rel, true)
            {
                continue;
            }
            walk_dir(root, path.as_path(), orbitignore, registry, out)?;
        } else if file_type.is_file() {
            if name.as_ref() == ORBITIGNORE_FILE_NAME {
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str())
                && SKIP_EXTENSIONS.contains(&ext)
            {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                if orbitignore.is_ignored(rel, false) || registry.language_for(rel).is_none() {
                    continue;
                }
                out.push(DiskFile {
                    path: rel.to_path_buf(),
                    mtime_ns: mtime_ns(path.as_path())?,
                });
            }
        }
    }

    Ok(())
}

fn collect_orbitignore_files(
    root: &Path,
    dir: &Path,
    default_orbitignore: &OrbitIgnoreMatcher,
    out: &mut Vec<PathBuf>,
) -> Result<(), GraphError> {
    let entries =
        fs::read_dir(dir).map_err(|source| GraphError::io("scan directory", dir, source))?;

    for entry in entries {
        let entry = entry.map_err(|source| GraphError::io("read directory entry", dir, source))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|source| GraphError::io("read file type", path.as_path(), source))?;

        if file_type.is_dir() {
            if let Ok(rel) = path.strip_prefix(root)
                && default_orbitignore.is_ignored(rel, true)
            {
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            collect_orbitignore_files(root, path.as_path(), default_orbitignore, out)?;
        } else if file_type.is_file() && name.as_ref() == ORBITIGNORE_FILE_NAME {
            let relative = path.strip_prefix(root).map_err(|error| {
                GraphError::invalid_data("strip .orbitignore prefix", error.to_string())
            })?;
            out.push(root.join(relative));
        }
    }

    Ok(())
}

fn git_ignored_paths(repo_path: &Path, paths: &[DiskFile]) -> HashSet<PathBuf> {
    let mut ignored = HashSet::new();
    if paths.is_empty() {
        return ignored;
    }

    let stdin_data = paths
        .iter()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");

    let output = Command::new("git")
        .args(["check-ignore", "--stdin"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(stdin_data.as_bytes());
            }
            child.wait_with_output()
        });

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                ignored.insert(PathBuf::from(trimmed));
            }
        }
    }

    ignored
}

fn hash_file(
    root: &Path,
    rel_path: &Path,
    hasher: &dyn ContentHasher,
) -> Result<Vec<u8>, GraphError> {
    let path = root.join(rel_path);
    let bytes = fs::read(path.as_path())
        .map_err(|source| GraphError::io("read file for content hash", path, source))?;
    Ok(hasher.hash(rel_path, &bytes))
}

pub(crate) fn mtime_ns(path: &Path) -> Result<i64, GraphError> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|source| GraphError::io("read file mtime", path, source))?;
    let duration = modified.duration_since(UNIX_EPOCH).map_err(|error| {
        GraphError::invalid_data(
            "read file mtime",
            format!("{} is before UNIX_EPOCH: {error}", path.display()),
        )
    })?;
    i64::try_from(duration.as_nanos()).map_err(|error| {
        GraphError::invalid_data(
            "read file mtime",
            format!("{} mtime is out of range: {error}", path.display()),
        )
    })
}

pub(crate) fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sort_diff(diff: &mut Diff) {
    diff.unchanged.sort();
    diff.modified.sort();
    diff.new.sort();
    diff.deleted.sort();
}

#[cfg(test)]
fn note_scan_started(worktree_root: &Path) {
    let mut counts = scan_counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *counts.entry(worktree_root.to_path_buf()).or_insert(0) += 1;
}

#[cfg(not(test))]
fn note_scan_started(_worktree_root: &Path) {}

#[cfg(test)]
fn scan_count(worktree_root: &Path) -> usize {
    scan_counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(worktree_root)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
fn scan_counts() -> &'static std::sync::Mutex<BTreeMap<PathBuf, usize>> {
    static SCAN_COUNTS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<PathBuf, usize>>> =
        std::sync::OnceLock::new();
    SCAN_COUNTS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
#[path = "tests/scanner.rs"]
mod tests;
