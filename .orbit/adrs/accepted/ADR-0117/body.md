## Context
Once fastembed-rs is the chosen backend (ADR-001), the question of where it lives matters. Linking ONNX Runtime + fastembed-rs into the main `orbit` binary adds ~50MB and pays that cost for every user — including users who never invoke semantic search. Three packaging shapes are plausible:

| Option | Default install size | Opt-in mechanism | Inference latency |
|--------|----------------------|------------------|-------------------|
| **A. Bundled in `orbit`** | Large (~50MB+) | None (always available) | In-process; instant after warm cache |
| **B. Cargo feature flag, two release artifacts** | Small or large depending on which artifact you download | Choose `orbit-full` at install time; replace the binary to swap | In-process; instant |
| **C. Companion binary downloaded on demand** | Small | `orbit semantic install [--model X]` | Subprocess; ~100–300ms ORT cold start, amortized across batches |

Option A is what the design originally called "single binary install posture preserved." It does preserve that, but it also means the always-pay binary cost is a permanent tax on users who don't want semantic search. Option B requires users to swap their main binary, which is gross UX (in-flight processes, partially-applied upgrades, surprising behavior changes). Option C keeps the default install slim and gives the user explicit control over which model — and how much disk — they're committing to, at the cost of subprocess overhead.

## Decision
Phase 1 ships option C. Two new crates:

- `orbit-embed` — small library holding the `Embedder` trait, JSON-RPC types, and `SubprocessEmbedder` (the trait impl that locates and talks to the companion). No fastembed-rs dependency. Linked into the main `orbit` binary.
- `orbit-embed-companion` — binary crate. Depends on `orbit-embed` + fastembed-rs. Produces a standalone `orbit-embed-companion` binary distributed via GitHub Releases per platform.

`orbit semantic install [--model bge-small | minilm-l6 | nomic-v1.5]` downloads the platform-appropriate companion binary plus the chosen model files into `~/.orbit/embed/`. Inference happens via stdio JSON-RPC; the subprocess is kept alive across a batch (`reindex`, multi-query session) and shut down at process exit. `orbit semantic uninstall` removes both the companion and the model. When semantic search is invoked without the companion installed, all read/write paths fail with a clear, actionable error pointing at `orbit semantic install`.

## Consequences
- Default `orbit` install stays slim — no ORT, no fastembed-rs in the main binary. Users who don't want semantic search pay no cost.
- The model menu is exposed at install time, not as a runtime config knob the user has to discover. Users actively choose between MiniLM-L6 (smallest, ~23MB), BGE-small (default, ~30MB), and Nomic-v1.5 (largest, ~140MB) at the moment they're committing to the feature.
- The subprocess-RPC boundary makes the companion swappable: a future `orbit-embed-companion-candle` could reuse the same RPC protocol with a different inference engine.
- Cost: install becomes a two-step user action (`orbit` install, then `orbit semantic install`). Users hitting `orbit semantic search` without the companion installed need a clean, helpful error. The subprocess introduces ~100–300ms ORT cold-start latency per process; mitigated by reusing the subprocess across batches but still visible on first interactive query. Additionally, the companion binary requires a per-platform release pipeline (Linux x86_64, Linux arm64, macOS x86_64, macOS arm64, Windows x86_64), which is real release-engineering work for follow-up tasks.

---
