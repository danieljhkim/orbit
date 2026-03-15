1. Locate the transition guard in orbit-core/src/command/task.rs (or wherever in-progress->review validation lives).
2. Change the hard error to a warning printed to stderr: 'warning: no execution_summary set; consider adding one with orbit task update --execution-summary'.
3. Allow the transition to proceed regardless.
4. Alternative: add a --skip-summary-check flag for programmatic callers that want strict enforcement.
5. Update relevant tests that assert on the error.