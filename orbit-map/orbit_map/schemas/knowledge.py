from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class ComponentV1(BaseModel):
    name: str
    role: str
    depends_on: list[str] = Field(default_factory=list)


class FlowV1(BaseModel):
    name: str
    description: str
    steps: list[str]


class ArchitectureV1(BaseModel):
    summary: str
    components: list[ComponentV1] = Field(default_factory=list)
    key_flows: list[FlowV1] = Field(default_factory=list)


class FileSymbolV1(BaseModel):
    name: str
    kind: Literal[
        "function",
        "struct",
        "class",
        "interface",
        "module",
        "method",
        "field",
        "trait",
        "impl",
    ]
    signature: str
    description: str


class FileMetadataV1(BaseModel):
    size_bytes: int
    last_modified: datetime


class FileSummaryV1(BaseModel):
    path: str
    hash: str
    language: str
    summary: str
    symbols: list[FileSymbolV1] = Field(default_factory=list)
    imports: list[str] = Field(default_factory=list)
    exports: list[str] = Field(default_factory=list)
    metadata: FileMetadataV1


class SourceFileV1(BaseModel):
    path: str
    hash: str
    language: str
    content: str
    metadata: FileMetadataV1


class FileSummaryAnalysisV1(BaseModel):
    summary: str = "Failed to summarize"
    symbols: list[FileSymbolV1] = Field(default_factory=list)
    imports: list[str] = Field(default_factory=list)
    exports: list[str] = Field(default_factory=list)


class SummarizeFilesInputV1(BaseModel):
    files: list[SourceFileV1] = Field(default_factory=list)


class SummarizeFilesResponseV1(BaseModel):
    files: list[FileSummaryV1] = Field(default_factory=list)


class GenerateArchitectureInputV1(BaseModel):
    file_summaries: list[FileSummaryV1] = Field(default_factory=list)


class GenerateArchitectureResponseV1(BaseModel):
    architecture: ArchitectureV1


class ArtifactsRef(BaseModel):
    architecture: str
    files_dir: str
    graph: str | None = None


class ManifestInputV1(BaseModel):
    repo_root: str
    artifacts: ArtifactsRef


class ManifestV1(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    schema_version: int = Field(default=1, alias="schemaVersion")
    generated_at: datetime
    repo_root: str
    artifacts: ArtifactsRef


class HashCacheV1(BaseModel):
    entries: dict[str, str]
