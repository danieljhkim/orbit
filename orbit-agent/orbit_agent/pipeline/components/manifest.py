from __future__ import annotations

import logging
from datetime import datetime, timezone
from pathlib import Path

from orbit_agent.pipeline.context import PipelineContext
from orbit_agent.schemas import ArtifactsRef, ManifestInputV1, ManifestV1

from .base import BaseComponent

logger = logging.getLogger(__name__)


class ManifestComponent(BaseComponent):
    name = "manifest"

    def _read(
        self,
        repo_path: Path,
        artifacts: ArtifactsRef | None = None,
    ) -> ManifestInputV1:
        return ManifestInputV1(
            repo_root=str(repo_path),
            artifacts=artifacts
            or ArtifactsRef(
                architecture="architecture.json",
                files_dir="files/",
                graph="graph/refs/current.json",
            ),
        )

    def _transform(self, data: ManifestInputV1) -> ManifestV1:
        return ManifestV1(
            generated_at=datetime.now(timezone.utc),
            repo_root=data.repo_root,
            artifacts=data.artifacts,
        )

    def _write(self, response: ManifestV1, output_dir: Path) -> None:
        output_dir.mkdir(parents=True, exist_ok=True)
        manifest_path = output_dir / "manifest.json"
        manifest_path.write_text(
            response.model_dump_json(indent=2, by_alias=True) + "\n"
        )

    def execute(self, context: PipelineContext) -> PipelineContext:
        logger.info("Writing manifest artifact")
        data = self._read(context.repo_path)
        response = self._transform(data)
        self._write(response, context.output_dir)
        context.manifest_response = response
        return context
