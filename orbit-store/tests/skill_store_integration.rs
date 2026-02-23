use orbit_store::Store;
use orbit_store::task_store::TaskInsertParams;
use orbit_types::{Role, Skill};

fn sample_skill(name: &str) -> Skill {
    let now = chrono::Utc::now();
    Skill {
        schema_version: 1,
        name: name.to_string(),
        description: Some("desc".to_string()),
        instructions: "instructions".to_string(),
        context_files: vec!["ARCHITECTURE.md".to_string()],
        allowed_tools: vec!["fs.read".to_string()],
        role: Role::Agent,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn legacy_skill_mutations_are_disabled() {
    let store = Store::open_in_memory().expect("store");
    let err = store
        .with_transaction(|tx| {
            tx.insert_skill(&sample_skill("legacy"))?;
            Ok(())
        })
        .expect_err("insert should fail");
    assert!(
        err.to_string()
            .contains("legacy sqlite skill mutation is disabled")
    );
}

#[test]
fn legacy_task_skill_attachment_is_disabled() {
    let store = Store::open_in_memory().expect("store");
    let task = store
        .with_transaction(|tx| {
            tx.insert_task(&TaskInsertParams {
                title: "task".to_string(),
                ..Default::default()
            })
        })
        .expect("task");

    let err = store
        .with_transaction(|tx| {
            tx.attach_skill_to_task(&task.id, "legacy")?;
            Ok(())
        })
        .expect_err("attach should fail");
    assert!(
        err.to_string()
            .contains("task-skill attachment is disabled")
    );
}
