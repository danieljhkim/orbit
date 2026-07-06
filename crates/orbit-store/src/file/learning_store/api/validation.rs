// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

// ORB-10046: this module previously validated learning comment/vote JSONL
// files during reindex. Both surfaces were removed; the file is preserved as
// an empty placeholder in case future reindex-side validators need a home.
