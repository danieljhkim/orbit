from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field

from orbit_map.schemas.graph.nodes import (
    DirNode,
    FileNode,
    LeafHistoryEntry,
    LeafKind,
    LeafNode,
)

NodeType = Literal["dir", "file", "leaf"]


class NodeContextRef(BaseModel):
    id: str
    name: str
    node_type: NodeType
    location: str
    language: str
    description: str = ""
    parent_id: str | None = None
    kind: LeafKind | None = None


class NodeLockState(BaseModel):
    is_locked: bool = False
    lineage_locked: bool = False
    lock_owner: str | None = None
    lock_reason: str = ""


class DirContext(BaseModel):
    node: DirNode
    lock: NodeLockState
    parent: NodeContextRef | None = None
    lineage: list[NodeContextRef] = Field(default_factory=list)
    child_dirs: list[NodeContextRef] = Field(default_factory=list)
    child_files: list[NodeContextRef] = Field(default_factory=list)
    summary: str = ""


class FileContext(BaseModel):
    node: FileNode
    lock: NodeLockState
    parent_dir: NodeContextRef | None = None
    lineage: list[NodeContextRef] = Field(default_factory=list)
    imports: list[str] = Field(default_factory=list)
    exports: list[str] = Field(default_factory=list)
    top_level_leaves: list[NodeContextRef] = Field(default_factory=list)
    summary: str = ""


class LeafContext(BaseModel):
    node: LeafNode
    lock: NodeLockState
    parent_file: NodeContextRef | None = None
    lineage: list[NodeContextRef] = Field(default_factory=list)
    child_leaves: list[NodeContextRef] = Field(default_factory=list)
    siblings: list[NodeContextRef] = Field(default_factory=list)
    history: list[LeafHistoryEntry] = Field(default_factory=list)
