from pathlib import Path

import pytest

from orbit_map.graph.extraction.base import GraphExtractionInput, source_hash
from orbit_map.graph.extraction.rust import RustGraphExtractor


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "rust"

FIXTURE_CASES = {
    "runtime_trait.rs": {
        "original_path": "crates/orbit-agent/src/runtime/runtime_trait.rs",
        "imports": [
            "use orbit_types::{InvocationTrace, OrbitError};",
            "use crate::types::{AgentRequest, AgentResponse};",
        ],
        "exports": ["AgentRuntime"],
        "top_level": [("AgentRuntime", "trait")],
        "leaves": [
            {
                "name": "AgentRuntime",
                "kind": "trait",
                "parent_kind": "file",
                "children_names": [],
                "start_line": 5,
                "end_line": 11,
            }
        ],
    },
    "lib.rs": {
        "original_path": "crates/orbit-types/src/lib.rs",
        "imports": [],
        "exports": [
            "activity",
            "actor",
            "agent_pair",
            "audit",
            "audit_event",
            "duel",
            "error",
            "event",
            "friction",
            "id",
            "invocation",
            "job",
            "metrics",
            "policy_decision",
            "redaction",
            "role",
            "skill",
            "task",
            "tool",
            "workspace",
        ],
        "top_level": [
            ("activity", "module"),
            ("actor", "module"),
            ("agent_pair", "module"),
            ("audit", "module"),
            ("audit_event", "module"),
            ("duel", "module"),
            ("error", "module"),
            ("event", "module"),
            ("friction", "module"),
            ("id", "module"),
            ("invocation", "module"),
            ("job", "module"),
            ("metrics", "module"),
            ("policy_decision", "module"),
            ("redaction", "module"),
            ("role", "module"),
            ("skill", "module"),
            ("task", "module"),
            ("tool", "module"),
            ("workspace", "module"),
        ],
        "leaves": [
            {"name": "activity", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 22, "end_line": 22},
            {"name": "actor", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 23, "end_line": 23},
            {"name": "agent_pair", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 24, "end_line": 24},
            {"name": "audit", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 25, "end_line": 25},
            {"name": "audit_event", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 26, "end_line": 26},
            {"name": "duel", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 27, "end_line": 27},
            {"name": "error", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 28, "end_line": 28},
            {"name": "event", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 29, "end_line": 29},
            {"name": "friction", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 30, "end_line": 30},
            {"name": "id", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 31, "end_line": 31},
            {"name": "invocation", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 32, "end_line": 32},
            {"name": "job", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 33, "end_line": 33},
            {"name": "metrics", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 34, "end_line": 34},
            {"name": "policy_decision", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 35, "end_line": 35},
            {"name": "redaction", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 36, "end_line": 36},
            {"name": "role", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 37, "end_line": 37},
            {"name": "skill", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 38, "end_line": 38},
            {"name": "task", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 39, "end_line": 39},
            {"name": "tool", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 40, "end_line": 40},
            {"name": "workspace", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 41, "end_line": 41},
        ],
    },
    "error.rs": {
        "original_path": "crates/orbit-types/src/error.rs",
        "imports": ["use thiserror::Error;"],
        "exports": ["OrbitError"],
        "top_level": [("OrbitError", "struct")],
        "leaves": [
            {
                "name": "OrbitError",
                "kind": "struct",
                "parent_kind": "file",
                "children_names": [],
                "start_line": 4,
                "end_line": 47,
            }
        ],
    },
    "evaluator.rs": {
        "original_path": "crates/orbit-policy/src/evaluator.rs",
        "imports": [
            "use std::collections::HashSet;",
            "use crate::PolicyDecision;",
            "use crate::engine::PolicyContext;",
            "use orbit_types::Role;",
        ],
        "exports": ["evaluate"],
        "top_level": [("evaluate", "function"), ("tests", "module")],
        "leaves": [
            {"name": "evaluate", "kind": "function", "parent_kind": "file", "children_names": [], "start_line": 7, "end_line": 28},
            {"name": "tests", "kind": "module", "parent_kind": "file", "children_names": [], "start_line": 31, "end_line": 115},
        ],
    },
    "audit_middleware.rs": {
        "original_path": "crates/orbit-cli/src/audit_middleware.rs",
        "imports": [
            "use std::time::Instant;",
            "use orbit_core::{\n    AuditEventInsertParams, AuditEventStatus, OrbitError, OrbitRuntime, redact_sensitive_env_text,\n};",
            "use crate::command::Commands;",
        ],
        "exports": ["CommandMeta", "AuditGuard", "extract_command_meta"],
        "top_level": [
            ("CommandMeta", "struct"),
            ("AuditGuard", "struct"),
            ("Drop", "impl"),
            ("extract_command_meta", "function"),
        ],
        "leaves": [
            {"name": "CommandMeta", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 9, "end_line": 17},
            {"name": "AuditGuard", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 34, "end_line": 42},
            {"name": "drop", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 82, "end_line": 123},
            {"name": "Drop", "kind": "impl", "parent_kind": "file", "children_names": ["drop"], "start_line": 81, "end_line": 124},
            {"name": "extract_command_meta", "kind": "function", "parent_kind": "file", "children_names": [], "start_line": 126, "end_line": 384},
        ],
    },
    "connection.rs": {
        "original_path": "crates/orbit-store/src/sqlite/connection.rs",
        "imports": [
            "use std::path::Path;",
            "use std::sync::{Arc, Mutex};",
            "use orbit_types::OrbitError;",
            "use rusqlite::{Connection, Transaction};",
            "use crate::sqlite::migration;",
        ],
        "exports": ["Store", "StoreTx"],
        "top_level": [
            ("Store", "struct"),
            ("StoreTx", "struct"),
            ("Store", "impl"),
            ("enable_best_effort_wal_mode", "function"),
            ("set_journal_mode_wal", "function"),
        ],
        "leaves": [
            {"name": "Store", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 10, "end_line": 12},
            {"name": "StoreTx", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 14, "end_line": 16},
            {"name": "open", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 19, "end_line": 35},
            {"name": "open_in_memory", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 37, "end_line": 45},
            {"name": "with_transaction", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 47, "end_line": 68},
            {"name": "connection", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 70, "end_line": 72},
            {"name": "Store", "kind": "impl", "parent_kind": "file", "children_names": ["open", "open_in_memory", "with_transaction", "connection"], "start_line": 18, "end_line": 73},
            {"name": "enable_best_effort_wal_mode", "kind": "function", "parent_kind": "file", "children_names": [], "start_line": 75, "end_line": 92},
            {"name": "set_journal_mode_wal", "kind": "function", "parent_kind": "file", "children_names": [], "start_line": 94, "end_line": 97},
        ],
    },
    "actor.rs": {
        "original_path": "crates/orbit-types/src/actor.rs",
        "imports": [
            "use std::fmt::{Display, Formatter};",
            "use serde::{Deserialize, Deserializer, Serialize, Serializer};",
        ],
        "exports": ["ActorIdentity"],
        "top_level": [
            ("ActorIdentity", "struct"),
            ("ActorIdentity", "impl"),
            ("Display", "impl"),
            ("Serialize", "impl"),
            ("AgentFields", "struct"),
        ],
        "leaves": [
            {"name": "ActorIdentity", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 11, "end_line": 19},
            {"name": "agent", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 23, "end_line": 28},
            {"name": "human", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 31, "end_line": 35},
            {"name": "from_legacy", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 42, "end_line": 57},
            {"name": "agent_name", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 60, "end_line": 65},
            {"name": "agent_model", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 68, "end_line": 73},
            {"name": "label", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 76, "end_line": 83},
            {"name": "is_agent", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 86, "end_line": 88},
            {"name": "is_system", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 91, "end_line": 93},
            {"name": "is_human", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 96, "end_line": 98},
            {"name": "to_legacy", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 102, "end_line": 115},
            {"name": "ActorIdentity", "kind": "impl", "parent_kind": "file", "children_names": ["agent", "human", "from_legacy", "agent_name", "agent_model", "label", "is_agent", "is_system", "is_human", "to_legacy"], "start_line": 21, "end_line": 116},
            {"name": "fmt", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 119, "end_line": 121},
            {"name": "Display", "kind": "impl", "parent_kind": "file", "children_names": ["fmt"], "start_line": 118, "end_line": 122},
            {"name": "serialize", "kind": "method", "parent_kind": "leaf", "children_names": [], "start_line": 130, "end_line": 132},
            {"name": "Serialize", "kind": "impl", "parent_kind": "file", "children_names": ["serialize"], "start_line": 129, "end_line": 133},
            {"name": "AgentFields", "kind": "struct", "parent_kind": "file", "children_names": [], "start_line": 136, "end_line": 139},
        ],
    },
}


def _extract_fixture(fixture_name: str):
    fixture = FIXTURE_CASES[fixture_name]
    source = (FIXTURE_ROOT / fixture_name).read_text(encoding="utf-8")
    file_hash = source_hash(source)
    file_id = f"file:{fixture['original_path']}"
    result = RustGraphExtractor().extract(
        GraphExtractionInput(
            path=fixture["original_path"],
            source=source,
            file_id=file_id,
            file_hash=file_hash,
        )
    )
    return result, file_id, file_hash


def _normalize_leaves(result, file_id: str) -> list[dict[str, object]]:
    leaf_by_id = {leaf.id: leaf for leaf in result.leaves}
    return [
        {
            "name": leaf.name,
            "kind": leaf.kind,
            "parent_kind": "file" if leaf.parent_id == file_id else "leaf",
            "children_names": [leaf_by_id[child_id].name for child_id in leaf.children],
            "start_line": leaf.start_line,
            "end_line": leaf.end_line,
        }
        for leaf in result.leaves
    ]


@pytest.mark.parametrize("fixture_name", FIXTURE_CASES)
def test_rust_extractor_matches_expected_fixture_graph(fixture_name: str):
    fixture = FIXTURE_CASES[fixture_name]
    result, file_id, file_hash = _extract_fixture(fixture_name)

    assert result.imports == fixture["imports"]
    assert result.exports == fixture["exports"]
    assert len(result.leaves) == len(fixture["leaves"])
    assert len(result.top_level_leaf_ids) == len(fixture["top_level"])

    top_level = [
        (leaf.name, leaf.kind) for leaf in result.leaves if leaf.parent_id == file_id
    ]
    assert top_level == fixture["top_level"]
    assert result.top_level_leaf_ids == [
        leaf.id for leaf in result.leaves if leaf.parent_id == file_id
    ]
    assert _normalize_leaves(result, file_id) == fixture["leaves"]

    for leaf in result.leaves:
        assert leaf.source_hash
        assert leaf.file_hash_at_capture == file_hash
        assert leaf.start_line is not None
        assert leaf.end_line is not None
        assert leaf.start_line >= 1
        assert leaf.end_line >= leaf.start_line
