# orbit-knowledge

`orbit-knowledge` contains Orbit's Rust-side knowledge graph logic: selector parsing,
knowledge pack resolution, working-graph mutation, and lightweight source extraction.
The built-in tree-sitter extractors currently cover Rust, Python, Go, Java, and
JavaScript source files.

The Python package in [`orbit-map/`](/Users/daniel/workspace/repos/orbit/orbit-map) is the
current knowledge-builder counterpart that writes the `.orbit/knowledge/` artifacts this
crate reads and augments. The long-term plan is to port that Python functionality into this
crate incrementally without changing the on-disk knowledge format.
