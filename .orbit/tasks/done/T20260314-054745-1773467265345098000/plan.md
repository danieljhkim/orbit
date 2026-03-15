1. Add --title <TITLE> option to TaskUpdateArgs in orbit-cli/src/command/task.rs.
2. Pass it through to TaskUpdateParams in orbit-core/src/command/task.rs.
3. Apply to the task YAML on update (same pattern as description/plan).
4. Add a test: orbit task update <id> --title 'New Title' then orbit task show <id> asserts title changed.
5. Guard: reject empty string (same as blank comment validation).