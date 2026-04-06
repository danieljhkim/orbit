from __future__ import annotations

import hashlib
from typing import Protocol

from pydantic import BaseModel, Field

from orbit_agent.schemas import LeafNode


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

    def extract(self, input_data: GraphExtractionInput) -> GraphExtractionResult:
        ...


def leaf_id(path: str, qualified_name: str, start_line: int | None) -> str:
    suffix = start_line if start_line is not None else "unknown"
    return f"leaf:{path}:{qualified_name}:{suffix}"


def source_hash(source: str) -> str | None:
    if not source:
        return None
    return hashlib.sha256(source.encode("utf-8")).hexdigest()
