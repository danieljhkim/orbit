from __future__ import annotations

import logging

from orbit_map.pipeline.context import PipelineContext
from orbit_map.pipeline.scan import scan_repo

from .base import BaseComponent

logger = logging.getLogger(__name__)


class ScanRepoComponent(BaseComponent):
    name = "scan_repo"

    def execute(self, context: PipelineContext) -> PipelineContext:
        logger.info("Scanning repository: %s", context.repo_path)
        context.file_paths = scan_repo(context.repo_path)
        logger.info("Scan complete: %d file(s) selected", len(context.file_paths))
        return context
