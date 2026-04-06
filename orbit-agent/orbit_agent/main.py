from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import get_args

import click

from orbit_agent.logging_utils import configure_logging
from orbit_agent.pipeline.components import DEFAULT_COMPONENT_NAMES
from orbit_agent.pipeline.config import PipelineConfig
from orbit_agent.pipeline.engine import run_build
from orbit_agent.pipeline.registry import build_default_registry
from orbit_agent.schemas import NodeContextRef
from orbit_agent.schemas.graph.contexts import NodeType
from orbit_agent.schemas.graph.nodes import LeafKind
from orbit_agent.service import GraphContextService

logger = logging.getLogger(__name__)

NODE_TYPE_CHOICES = ("dir", "file", "leaf")
LEAF_KIND_CHOICES = tuple(str(value) for value in get_args(LeafKind))


@click.group()
@click.option("--debug", is_flag=True, help="Enable debug logging.")
@click.pass_context
def cli(ctx: click.Context, debug: bool) -> None:
    configure_logging(debug=debug)
    ctx.ensure_object(dict)
    ctx.obj["debug"] = debug
    logger.debug("CLI initialized with debug=%s", debug)


@cli.command()
@click.option("--repo", default=".", help="Repository root path.")
@click.option("--output", default=".orbit/knowledge", help="Output directory.")
@click.option(
    "--components",
    default=",".join(DEFAULT_COMPONENT_NAMES),
    help="Comma-separated ordered component names.",
)
def build(repo: str, output: str, components: str) -> None:
    """Scan and build full knowledge base."""
    repo_path, output_dir = _resolve_paths(repo, output)
    logger.info("Starting full knowledge build for %s", repo_path)
    run_build(
        repo_path,
        output_dir,
        incremental=False,
        config=_parse_pipeline_config(components),
    )
    click.echo(f"Knowledge artifacts written to {output_dir}")


@cli.command()
@click.option("--repo", default=".", help="Repository root path.")
@click.option("--output", default=".orbit/knowledge", help="Output directory.")
@click.option(
    "--components",
    default=",".join(DEFAULT_COMPONENT_NAMES),
    help="Comma-separated ordered component names.",
)
def update(repo: str, output: str, components: str) -> None:
    """Incrementally update knowledge base."""
    repo_path, output_dir = _resolve_paths(repo, output)
    logger.info("Starting incremental knowledge update for %s", repo_path)
    run_build(
        repo_path,
        output_dir,
        incremental=True,
        config=_parse_pipeline_config(components),
    )
    click.echo(f"Knowledge artifacts updated at {output_dir}")


@cli.command("list-components")
def list_components() -> None:
    """List registered pipeline component names."""
    registry = build_default_registry()
    logger.debug("Listing %d registered components", len(registry.names()))
    for name in registry.names():
        click.echo(name)


@cli.group("graph")
def graph() -> None:
    """Inspect the persisted code graph."""


@graph.command("context")
@click.argument("node_id")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
def graph_context(node_id: str, repo: str, output: str) -> None:
    """Print an agent-facing context for a graph node."""
    service = _load_graph_context_service(repo, output)
    click.echo(service.get_context(node_id).model_dump_json(indent=2))


@graph.command("lineage")
@click.argument("node_id")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
@click.option(
    "--include-self", is_flag=True, help="Include the requested node in the lineage."
)
def graph_lineage(node_id: str, repo: str, output: str, include_self: bool) -> None:
    """Print the lineage for a graph node."""
    service = _load_graph_context_service(repo, output)
    _echo_refs(service.get_lineage(node_id, include_self=include_self))


@graph.command("children")
@click.argument("node_id")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
def graph_children(node_id: str, repo: str, output: str) -> None:
    """Print immediate child nodes for a graph node."""
    service = _load_graph_context_service(repo, output)
    _echo_refs(service.get_children(node_id))


@graph.command("search")
@click.argument("query", required=False, default="")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
@click.option(
    "--node-type",
    "node_types",
    multiple=True,
    type=click.Choice(NODE_TYPE_CHOICES),
    help="Filter by node type. May be used multiple times.",
)
@click.option(
    "--leaf-kind",
    "leaf_kinds",
    multiple=True,
    type=click.Choice(LEAF_KIND_CHOICES),
    help="Filter leaf nodes by kind. May be used multiple times.",
)
@click.option(
    "--location-prefix", default=None, help="Filter by graph node location prefix."
)
@click.option(
    "--limit", default=20, show_default=True, help="Maximum number of search results."
)
def graph_search(
    query: str,
    repo: str,
    output: str,
    node_types: tuple[NodeType, ...],
    leaf_kinds: tuple[LeafKind, ...],
    location_prefix: str | None,
    limit: int,
) -> None:
    """Search graph nodes and print lightweight references."""
    service = _load_graph_context_service(repo, output)
    _echo_refs(
        service.search_nodes(
            query=query,
            node_types=node_types or None,
            leaf_kinds=leaf_kinds or None,
            location_prefix=location_prefix,
            limit=limit,
        )
    )


def _resolve_paths(repo: str, output: str) -> tuple[Path, Path]:
    repo_path = Path(repo).resolve()
    output_dir = Path(output) if Path(output).is_absolute() else repo_path / output
    logger.debug("Resolved repo_path=%s output_dir=%s", repo_path, output_dir)
    return repo_path, output_dir


def _parse_pipeline_config(components: str) -> PipelineConfig:
    component_names = [name.strip() for name in components.split(",") if name.strip()]
    logger.debug("Parsed component names: %s", component_names)
    return PipelineConfig.from_component_names(component_names)


def _load_graph_context_service(repo: str, output: str) -> GraphContextService:
    _, output_dir = _resolve_paths(repo, output)
    logger.debug("Loading graph context service from %s", output_dir)
    return GraphContextService.from_knowledge_dir(output_dir)


def _echo_refs(refs: list[NodeContextRef]) -> None:
    click.echo(json.dumps([ref.model_dump(mode="json") for ref in refs], indent=2))


if __name__ == "__main__":
    cli()
