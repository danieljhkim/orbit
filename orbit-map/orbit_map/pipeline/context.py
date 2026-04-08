from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from orbit_map.runtime.agent.base import BaseAgent
from orbit_map.schemas import (
    CodebaseGraphV1,
    GenerateArchitectureResponseV1,
    ManifestV1,
    SummarizeFilesResponseV1,
)


@dataclass
class PipelineContext:
    repo_path: Path
    output_dir: Path
    incremental: bool = False
    agent: BaseAgent | None = None
    file_paths: list[Path] = field(default_factory=list)
    new_hashes: dict[str, str] = field(default_factory=dict)
    changed_paths: list[str] = field(default_factory=list)
    codebase_graph: CodebaseGraphV1 | None = None
    summarize_response: SummarizeFilesResponseV1 | None = None
    architecture_response: GenerateArchitectureResponseV1 | None = None
    manifest_response: ManifestV1 | None = None

    @property
    def files_dir(self) -> Path:
        return self.output_dir / "files"

    @property
    def graph_dir(self) -> Path:
        return self.output_dir / "graph"
