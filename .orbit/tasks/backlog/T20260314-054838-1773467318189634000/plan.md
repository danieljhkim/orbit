1. Locate comment validation in orbit-cli/src/command/task.rs or the core update handler.
2. Add a .trim().is_empty() check alongside the existing empty-string check.
3. Return the same 'comment must not be blank' error for whitespace-only input.
4. Add a test: orbit task update <id> --comment '   ' asserts failure with the blank comment error.