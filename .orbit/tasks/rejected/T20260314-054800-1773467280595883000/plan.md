1. Add 'edit' subcommand to TaskSubcommand in orbit-cli.
2. Serialize the editable task fields to a temp file in a human-friendly format (TOML or YAML with comments).
3. Open $EDITOR (fallback: $VISUAL, then 'vi').
4. On save/exit, parse the temp file and apply changed fields via the existing update path.
5. Abort with no changes if the file is unmodified or the editor exits non-zero.
6. Fields in scope for editing: title, description, plan, assigned-to.