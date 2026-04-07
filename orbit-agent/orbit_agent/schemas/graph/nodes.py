from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field

LeafKind = Literal[
    "function",
    "method",
    "class",
    "struct",
    "interface",
    "trait",
    "impl",
    "field",
    "module",
]


class BaseNode(BaseModel):
    id: str
    identity_key: str = ""
    object_hash: str | None = None
    name: str
    location: str
    language: str
    description: str = ""
    parent_id: str | None = None
    is_locked: bool = False
    lineage_locked: bool = False
    lock_owner: str | None = None
    lock_reason: str = ""


class SignatureField(BaseModel):
    name: str
    annotation: str | None = None
    description: str = ""


class LeafHistoryEntry(BaseModel):
    timestamp: datetime
    actor: str
    reason: str = ""
    source: str = ""
    source_blob_hash: str | None = None
    source_hash: str | None = None
    file_hash_at_capture: str | None = None


class DirNode(BaseNode):
    node_type: Literal["dir"] = "dir"
    dir_children: list[str] = Field(default_factory=list)
    file_children: list[str] = Field(default_factory=list)


class FileNode(BaseNode):
    node_type: Literal["file"] = "file"
    extension: str | None = None
    source_blob_hash: str | None = None
    imports: list[str] = Field(default_factory=list)
    exports: list[str] = Field(default_factory=list)
    leaf_children: list[str] = Field(default_factory=list)


class LeafNode(BaseNode):
    node_type: Literal["leaf"] = "leaf"
    kind: LeafKind
    source: str = ""
    source_blob_hash: str | None = None
    source_hash: str | None = None
    file_hash_at_capture: str | None = None
    history: list[LeafHistoryEntry] = Field(default_factory=list)
    input_signature: list[SignatureField] = Field(default_factory=list)
    output_signature: list[SignatureField] = Field(default_factory=list)
    start_line: int | None = None
    end_line: int | None = None
    children: list[str] = Field(default_factory=list)


class CodebaseGraphV1(BaseModel):
    root_dir_id: str
    dirs: list[DirNode] = Field(default_factory=list)
    files: list[FileNode] = Field(default_factory=list)
    leaves: list[LeafNode] = Field(default_factory=list)
