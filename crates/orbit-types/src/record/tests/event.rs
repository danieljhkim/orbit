//! [ORB-10965] Audit reads events through their serialized tag, so the shape of
//! the deduplicated-start record is the contract.

use serde_json::json;

use crate::record::OrbitEvent;

#[test]
fn deduplicated_start_serializes_as_its_own_event_type() {
    let event = OrbitEvent::JobRunStartDeduplicated {
        job_id: "job-a".to_string(),
        run_id: "jrun-a".to_string(),
        attempt: 2,
        reason: "another worker owns the run".to_string(),
    };

    let payload = serde_json::to_value(&event).expect("serialize");
    assert_eq!(
        payload,
        json!({
            "type": "JobRunStartDeduplicated",
            "data": {
                "job_id": "job-a",
                "run_id": "jrun-a",
                "attempt": 2,
                "reason": "another worker owns the run",
            }
        }),
        "a deduplicated start is recorded on its own, never as a run failure"
    );
    assert_eq!(
        serde_json::from_value::<OrbitEvent>(payload).expect("round trip"),
        event
    );
}
