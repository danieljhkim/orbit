from __future__ import annotations

import logging

from orbit_map.pipeline.context import PipelineContext
from orbit_map.pipeline.hash import compute_hashes, detect_changes, save_hash_cache

from .base import BaseComponent

logger = logging.getLogger(__name__)


class ComputeHashesComponent(BaseComponent):
    name = "compute_hashes"

    def execute(self, context: PipelineContext) -> PipelineContext:
        logger.info("Computing hashes for %d file(s)", len(context.file_paths))
        context.new_hashes = compute_hashes(context.file_paths, context.repo_path)
        logger.info("Computed %d file hash(es)", len(context.new_hashes))
        return context


class SelectChangedPathsComponent(BaseComponent):
    name = "select_changed_paths"

    def execute(self, context: PipelineContext) -> PipelineContext:
        if context.incremental:
            logger.info("Selecting changed paths using incremental mode")
            context.changed_paths = detect_changes(
                context.new_hashes, context.output_dir
            )
        else:
            logger.info("Selecting all paths for full rebuild")
            context.changed_paths = sorted(context.new_hashes.keys())
        logger.info("Selected %d path(s) for summarization", len(context.changed_paths))
        return context


class SaveHashCacheComponent(BaseComponent):
    name = "save_hash_cache"

    def execute(self, context: PipelineContext) -> PipelineContext:
        logger.info("Saving hash cache for %d file(s)", len(context.new_hashes))
        save_hash_cache(context.new_hashes, context.output_dir)
        return context
