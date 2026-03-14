Option A: Rename the Cargo package from 'orbit' to 'orbit-cli' in orbit-cli/Cargo.toml. Update any references to the package name.
Option B: Document the naming exception clearly in CLAUDE.md under 'Running a Single Test'.
Recommendation: Option A is cleaner. Verify that the binary name ('orbit') can remain independent of the package name by using [[bin]] name = 'orbit' in Cargo.toml.