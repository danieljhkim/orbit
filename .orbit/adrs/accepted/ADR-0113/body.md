## Context
Local embedding inference has four plausible backends:

| Backend | Profile |
|---------|---------|
| **fastembed-rs** | Pure Rust crate wrapping ONNX Runtime; ships a small set of well-known sentence-embedding models (BGE, MiniLM, Nomic, mxbai); CPU-only fine; batch-friendly. |
| **Candle** | Pure-Rust ML framework from HuggingFace; broader model support; more code to integrate; less plug-and-play for embeddings specifically. |
| **llama-cpp-rs** | Bindings to llama.cpp; GGUF format; runs anything from tiny embedding models to large LLMs; optional GPU; C++ build dependency. |
| **External ollama or similar always-on daemon** | Outsources inference but requires the user to install and run a separate long-lived process. |

This ADR addresses *which* backend to use. The orthogonal decision of *how* the backend is delivered to the user (in-process vs. companion binary vs. feature flag) is in [ADR-005](#adr-005--companion-binary-installed-on-demand-rather-than-bundled-in-orbit). Within in-process or in-companion options, fastembed-rs covers the embedding-model use case directly; Candle is more general but requires more Orbit-side code; llama-cpp-rs is overkill and adds a C++ build dependency that complicates Orbit's release pipeline. An always-on ollama-style daemon contradicts Orbit's no-daemon posture regardless of binary placement.

## Decision
Phase 1 uses fastembed-rs as the inference backend, exposed through an `Embedder` trait that lives in a new `orbit-embed` library crate. Per ADR-005, fastembed-rs is linked into a separate `orbit-embed-companion` binary, not into the main `orbit` binary; the trait abstraction means an alternative backend can later swap in without touching `orbit-store` or `orbit-tools`. The user-facing default model is BGE-small-en-v1.5 (384 dim, ~30MB), with `--model {bge-small | minilm-l6 | nomic-v1.5}` selected at install time. Reject external always-on ollama: contradicts the no-daemon posture. Reject llama-cpp-rs: C++ build dependency outweighs its flexibility for embedding-only work. Reject Candle as default: more integration work for less out-of-the-box behavior; remains a viable trait-impl swap.

## Consequences
- The `Embedder` trait isolates the choice of backend from storage and retrieval; later-arriving backends (Candle, code-tuned models) plug in without schema or query changes.
- The fastembed-rs model catalog (BGE, MiniLM, Nomic, mxbai) is the menu phase-1 users pick from. Other model families require a new `Embedder` impl, not a config change.
- Model output is well-characterized by published benchmarks (MTEB) so the default is defensible without an Orbit-specific eval ([3_vision.md §1.1](./3_vision.md)).
- Cost: locking in to the fastembed-rs catalog means models outside that catalog (e.g., voyage-code, code-tuned models in [3_vision.md §1.7](./3_vision.md)) need a different `Embedder` impl in a future task. The trait abstraction makes that mechanical, but it does mean the phase-1 menu is bounded by what fastembed-rs ships.

---
