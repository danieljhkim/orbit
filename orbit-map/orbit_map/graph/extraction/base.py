from __future__ import annotations

import hashlib
from typing import Protocol

from pydantic import BaseModel, Field

from orbit_map.schemas import LeafNode
from orbit_map.schemas.graph.nodes import LeafKind


class GraphExtractionInput(BaseModel):
    path: str
    source: str
    file_id: str
    file_hash: str | None = None


class GraphExtractionResult(BaseModel):
    imports: list[str] = Field(default_factory=list)
    exports: list[str] = Field(default_factory=list)
    leaves: list[LeafNode] = Field(default_factory=list)
    top_level_leaf_ids: list[str] = Field(default_factory=list)


class GraphExtractor(Protocol):
    language: str

    def extract(self, input_data: GraphExtractionInput) -> GraphExtractionResult: ...


def identity_key(node_type: str, location: str, kind: str) -> str:
    return f"{node_type}:{location}:{kind}"


def node_id(node_type: str, location: str, kind: str) -> str:
    digest = hashlib.sha256(identity_key(node_type, location, kind).encode("utf-8"))
    return f"{node_type}:{digest.hexdigest()}"


def leaf_location(path: str, qualified_name: str) -> str:
    return f"{path}#{qualified_name}"


def leaf_identity_key(path: str, qualified_name: str, kind: LeafKind) -> str:
    return identity_key("leaf", leaf_location(path, qualified_name), kind)


def leaf_id(path: str, qualified_name: str, kind: LeafKind) -> str:
    return node_id("leaf", leaf_location(path, qualified_name), kind)


def source_hash(source: str) -> str | None:
    if not source:
        return None
    return hashlib.sha256(source.encode("utf-8")).hexdigest()
