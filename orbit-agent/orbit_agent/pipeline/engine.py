from __future__ import annotations

import logging
from pathlib import Path
from typing import Sequence

from orbit_agent.pipeline.components import BaseComponent, DEFAULT_COMPONENT_NAMES
from orbit_agent.pipeline.config import PipelineConfig
from orbit_agent.pipeline.context import PipelineContext
from orbit_agent.pipeline.registry import ComponentRegistry, build_default_registry

logger = logging.getLogger(__name__)


def _build_default_components(registry: ComponentRegistry) -> list[BaseComponent]:
    logger.debug("Building default component list: %s", DEFAULT_COMPONENT_NAMES)
    return registry.create_many(
        PipelineConfig.from_component_names(DEFAULT_COMPONENT_NAMES)
    )


def run_build(
    repo_path: Path,
    output_dir: Path,
    incremental: bool = False,
    components: Sequence[BaseComponent] | None = None,
    config: PipelineConfig | None = None,
    registry: ComponentRegistry | None = None,
) -> PipelineContext:
    component_registry = registry or build_default_registry()
    context = PipelineContext(
        repo_path=repo_path,
        output_dir=output_dir,
        incremental=incremental,
    )

    resolved_components = (
        list(components)
        if components is not None
        else (
            component_registry.create_many(config)
            if config is not None
            else _build_default_components(component_registry)
        )
    )

    logger.info(
        "Running knowledge pipeline for %s with %d component(s)",
        repo_path,
        len(resolved_components),
    )
    logger.debug(
        "Pipeline component order: %s",
        [component.name for component in resolved_components],
    )

    for component in resolved_components:
        logger.info("Executing component: %s", component.name)
        context = component.execute(context)
        logger.debug("Completed component: %s", component.name)

    logger.info("Knowledge pipeline completed for %s", repo_path)
    return context
