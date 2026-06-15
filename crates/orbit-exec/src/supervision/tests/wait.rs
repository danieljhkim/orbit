use std::time::{Duration, Instant};

use orbit_common::utility::output_capture::OUTPUT_TRUNCATED_MARKER;

use super::super::wait::wait_with_timeout_and_output_limit;
use crate::runner::{EnvironmentMode, ExecRequest, StdinMode};

#[cfg(unix)]
#[test]
fn wait_kills_process_when_stdout_capture_limit_is_exceeded() {
    let req = ExecRequest {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), "yes orbit-cap".to_string()],
        current_dir: None,
        timeout_ms: Some(5_000),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    };
    let child = crate::process::spawn(&req).expect("spawn child");

    let started = Instant::now();
    let result =
        wait_with_timeout_and_output_limit(child, Some(5_000), false, None, 64).expect("wait");

    assert!(!result.exit_success);
    // Hitting the output cap terminates the process two valid ways, decided by
    // thread scheduling: the supervisor may observe the cap signal first and
    // SIGKILL the group (exit_code None), or — under load — be parked in
    // child.wait_timeout when /bin/sh self-exits 141 because `yes` took SIGPIPE
    // the instant the drain thread dropped the pipe read end (128 + 13 = 141).
    // Both promptly kill the process and cap output (asserted below); only the
    // reported exit_code differs by who delivered the kill.
    assert!(
        result.exit_code.is_none() || result.exit_code == Some(141),
        "expected None (SIGKILL) or Some(141) (SIGPIPE cascade), got {:?}",
        result.exit_code
    );
    assert!(result.stderr.is_empty());
    assert!(result.stdout.ends_with(OUTPUT_TRUNCATED_MARKER));
    assert!(result.stdout.len() <= 64 + OUTPUT_TRUNCATED_MARKER.len());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "output cap should kill the subprocess promptly"
    );
}
