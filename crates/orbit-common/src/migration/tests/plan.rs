use super::super::*;
use serde_yaml::Mapping;

fn doc(version: u64) -> Value {
    let mut map = Mapping::new();
    map.insert(
        Value::String("schema_version".to_string()),
        Value::Number(version.into()),
    );
    Value::Mapping(map)
}

fn bump(mut value: Value) -> Result<Value, OrbitError> {
    let map = value.as_mapping_mut().expect("mapping");
    let current = map
        .get(Value::String("schema_version".to_string()))
        .and_then(Value::as_u64)
        .expect("schema_version");
    map.insert(
        Value::String("schema_version".to_string()),
        Value::Number((current + 1).into()),
    );
    Ok(value)
}

#[test]
fn no_op_when_already_at_target() {
    let plan = Plan::new("kind", 3).add_step(1, bump).add_step(2, bump);
    let migrated = plan.migrate(doc(3)).expect("no-op");
    assert_eq!(read_schema_version(&migrated).unwrap(), 3);
}

#[test]
fn applies_chain_in_order() {
    fn add_alpha(mut value: Value) -> Result<Value, OrbitError> {
        let map = value.as_mapping_mut().unwrap();
        map.insert(
            Value::String("alpha".to_string()),
            Value::String("from-v1".to_string()),
        );
        bump(value)
    }
    fn add_beta(mut value: Value) -> Result<Value, OrbitError> {
        let map = value.as_mapping_mut().unwrap();
        map.insert(
            Value::String("beta".to_string()),
            Value::String("from-v2".to_string()),
        );
        bump(value)
    }
    let plan = Plan::new("kind", 3)
        .add_step(1, add_alpha)
        .add_step(2, add_beta);

    let migrated = plan.migrate(doc(1)).expect("chain");
    let map = migrated.as_mapping().unwrap();
    assert_eq!(
        map.get(Value::String("alpha".to_string()))
            .and_then(Value::as_str),
        Some("from-v1")
    );
    assert_eq!(
        map.get(Value::String("beta".to_string()))
            .and_then(Value::as_str),
        Some("from-v2")
    );
    assert_eq!(read_schema_version(&migrated).unwrap(), 3);
}

#[test]
fn rejects_newer_than_target() {
    let plan = Plan::new("kind", 2).add_step(1, bump);
    let err = plan.migrate(doc(5)).expect_err("reject newer");
    let msg = err.to_string();
    assert!(msg.contains("newer than supported target"), "{msg}");
    assert!(msg.contains("kind"), "{msg}");
}

#[test]
fn rejects_missing_chain_link() {
    let plan = Plan::new("kind", 3).add_step(2, bump); // missing 1 -> 2
    let err = plan.migrate(doc(1)).expect_err("missing step");
    assert!(
        err.to_string().contains("missing migration step from v1"),
        "{err}"
    );
}

#[test]
fn rejects_step_that_does_not_bump_version() {
    fn forgetful(value: Value) -> Result<Value, OrbitError> {
        Ok(value)
    }
    let plan = Plan::new("kind", 2).add_step(1, forgetful);
    let err = plan.migrate(doc(1)).expect_err("no bump");
    assert!(
        err.to_string()
            .contains("produced schema_version 1, expected 2"),
        "{err}"
    );
}

#[test]
fn rejects_step_that_overshoots() {
    fn doubler(mut value: Value) -> Result<Value, OrbitError> {
        let map = value.as_mapping_mut().unwrap();
        map.insert(
            Value::String("schema_version".to_string()),
            Value::Number(99u64.into()),
        );
        Ok(value)
    }
    let plan = Plan::new("kind", 3).add_step(1, doubler);
    let err = plan.migrate(doc(1)).expect_err("overshoot");
    assert!(
        err.to_string()
            .contains("produced schema_version 99, expected 2"),
        "{err}"
    );
}

#[test]
fn propagates_step_error() {
    fn fails(_: Value) -> Result<Value, OrbitError> {
        Err(OrbitError::Migration("intentional".to_string()))
    }
    let plan = Plan::new("kind", 2).add_step(1, fails);
    let err = plan.migrate(doc(1)).expect_err("step error");
    assert!(err.to_string().contains("intentional"), "{err}");
}

#[test]
fn rejects_missing_schema_version_field() {
    let plan = Plan::new("kind", 1);
    let mut map = Mapping::new();
    map.insert(
        Value::String("id".to_string()),
        Value::String("ORB-00001".to_string()),
    );
    let err = plan.migrate(Value::Mapping(map)).expect_err("no field");
    assert!(err.to_string().contains("missing schema_version"), "{err}");
}

#[test]
fn rejects_non_mapping_root() {
    let plan = Plan::new("kind", 1);
    let err = plan
        .migrate(Value::String("not a map".to_string()))
        .expect_err("non-mapping");
    assert!(err.to_string().contains("expected YAML mapping"), "{err}");
}

#[test]
#[should_panic(expected = "would land at or past target")]
fn rejects_step_registered_at_target() {
    let _ = Plan::new("kind", 2).add_step(2, bump);
}

#[test]
#[should_panic(expected = "duplicate step")]
fn rejects_duplicate_step() {
    let _ = Plan::new("kind", 3).add_step(1, bump).add_step(1, bump);
}
