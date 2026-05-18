## Context
The Orbit web dashboard lived inside orbit-cli even though its HTML, JavaScript, read-only axum API handlers, and embedded assets formed a distinct internal surface. The only local coupling was the CLI Execute trait; keeping the dashboard in orbit-cli forced unrelated CLI edits to rebuild the heavier web tree and mixed dashboard tests into the CLI target.

## Decision
Extract the dashboard assets, ServeArgs, JSON API handlers, router construction, browser opener, and serve(runtime, args) entrypoint into the internal orbit-dashboard crate. Keep orbit-cli as a thin delegator that wires the clap subcommand to orbit_dashboard::serve.

## Consequences
- orbit-cli no longer carries the direct axum dashboard dependency for command-only edits.
- Dashboard assets live beside the Rust server that embeds them, and dashboard tests compile under a dedicated crate.
- No single Rust code anchor; this is a crate-boundary decision enforced through architecture review.
- Cost: one more workspace crate and temporary duplication of a few projection helpers until a later shared projection layer exists.