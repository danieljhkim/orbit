use super::super::*;

#[test]
fn validates_and_formats_orb_task_ids() {
    assert!(is_valid_orb_task_id("ORB-00000"));
    assert!(is_valid_orb_task_id("ORB-99999"));
    assert!(!is_valid_orb_task_id("ORB-100000"));
    assert!(!is_valid_orb_task_id("orb-00001"));
    assert_eq!(format_orb_task_id(42).unwrap(), "ORB-00042");
    assert!(format_orb_task_id(100_000).is_err());
    assert!(validate_orb_task_id("ORB-12345").is_ok());
    assert!(validate_orb_task_id("ORB-1234").is_err());
}
