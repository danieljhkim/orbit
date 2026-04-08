from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import get_args

import click

from orbit_map.logging_utils import configure_logging
from orbit_map.pipeline.config import PipelineConfig
from orbit_map.pipeline.engine import run_build
from orbit_map.schemas import NodeContextRef
from orbit_map.schemas.graph.contexts import NodeType
from orbit_map.schemas.graph.nodes import LeafKind
from orbit_map.service.bootstrap import render_knowledge_bootstrap
from orbit_map.service.lineage_pack import render_lineage_pack
from orbit_map.service import GraphContextService

logger = logging.getLogger(__name__)

NODE_TYPE_CHOICES = ("dir", "file", "leaf")
LEAF_KIND_CHOICES = tuple(str(value) for value in get_args(LeafKind))
BUILD_TARGET_CHOICES = ("graph", "knowledge")
BOOTSTRAP_FORMAT_CHOICES = ("markdown", "json")
PACK_FORMAT_CHOICES = ("markdown", "json")
GRAPH_COMPONENT_NAMES = [
    "scan_repo",
    "compute_hashes",
    "build_graph_dirs",
    "build_graph_files",
    "build_graph_leaves",
    "persist_graph",
    "manifest",
    "save_hash_cache",
]
KNOWLEDGE_COMPONENT_NAMES = [
    "summarize_files",
    "manifest",
]
GRAPH_AND_KNOWLEDGE_COMPONENT_NAMES = [
    "scan_repo",
    "compute_hashes",
    "build_graph_dirs",
    "build_graph_files",
    "build_graph_leaves",
    "persist_graph",
    "summarize_files",
    "manifest",
    "save_hash_cache",
]


@click.group()
@click.option("--debug", is_flag=True, help="Enable debug logging.")
@click.pass_context
def cli(ctx: click.Context, debug: bool) -> None:
    configure_logging(debug=debug)
    ctx.ensure_object(dict)
    ctx.obj["debug"] = debug
    logger.debug("CLI initialized with debug=%s", debug)


@cli.command()
@click.argument("target", type=click.Choice(BUILD_TARGET_CHOICES))
@click.option("--repo", default=".", help="Repository root path.")
@click.option("--output", default=".orbit/knowledge", help="Output directory.")
def build(target: str, repo: str, output: str) -> None:
    """Build a knowledge artifact."""
    repo_path, output_dir = _resolve_paths(repo, output)
    component_names = _build_component_names(target, output_dir)
    target_label = _target_label(target)
    logger.info("Starting %s build for %s", target_label, repo_path)
    run_build(
        repo_path,
        output_dir,
        incremental=False,
        config=PipelineConfig.from_component_names(component_names),
    )
    click.echo(f"{target_label.title()} artifacts written to {output_dir}")


@cli.command()
@click.argument("target", type=click.Choice(BUILD_TARGET_CHOICES))
@click.option("--repo", default=".", help="Repository root path.")
@click.option("--output", default=".orbit/knowledge", help="Output directory.")
def update(target: str, repo: str, output: str) -> None:
    """Update a knowledge artifact."""
    repo_path, output_dir = _resolve_paths(repo, output)
    component_names = _build_component_names(target, output_dir)
    target_label = _target_label(target)
    logger.info("Starting %s update for %s", target_label, repo_path)
    run_build(
        repo_path,
        output_dir,
        incremental=True,
        config=PipelineConfig.from_component_names(component_names),
    )
    click.echo(f"{target_label.title()} artifacts updated at {output_dir}")


@cli.group("knowledge")
def knowledge() -> None:
    """Render knowledge-oriented views from persisted artifacts."""


@knowledge.command("bootstrap")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
@click.option(
    "--format",
    "output_format",
    type=click.Choice(BOOTSTRAP_FORMAT_CHOICES),
    default="markdown",
    show_default=True,
)
@click.option(
    "--budget",
    type=int,
    default=12000,
    show_default=True,
    help="Approximate markdown character budget for the rendered bootstrap.",
)
@click.option(
    "--include-source",
    is_flag=True,
    help="Include source excerpts for leaf nodes when available.",
)
@click.option(
    "--source-budget",
    type=int,
    default=0,
    show_default=True,
    help="Maximum characters of leaf source excerpts to include.",
)
def knowledge_bootstrap(
    repo: str,
    output: str,
    output_format: str,
    budget: int,
    include_source: bool,
    source_budget: int,
) -> None:
    """Render a deterministic whole-codebase bootstrap."""
    _, output_dir = _resolve_paths(repo, output)
    logger.info("Rendering knowledge bootstrap from %s", output_dir)
    click.echo(
        render_knowledge_bootstrap(
            output_dir,
            format=output_format,
            budget=budget,
            include_source=include_source,
            source_budget=source_budget,
        )
    )


@knowledge.command("pack")
@click.argument("selectors", nargs=-1)
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
@click.option(
    "--format",
    "output_format",
    type=click.Choice(PACK_FORMAT_CHOICES),
    default="markdown",
    show_default=True,
)
@click.option(
    "--depth",
    type=int,
    default=2,
    show_default=True,
    help="Maximum ancestor lineage depth to include for each selected node.",
)
@click.option(
    "--siblings",
    type=int,
    default=2,
    show_default=True,
    help="Maximum sibling nodes to include around each focus node.",
)
@click.option(
    "--children",
    type=int,
    default=4,
    show_default=True,
    help="Maximum direct child nodes to include around each focus node.",
)
@click.option(
    "--budget",
    type=int,
    default=12000,
    show_default=True,
    help="Approximate markdown character budget for the rendered pack.",
)
@click.option(
    "--detail",
    is_flag=True,
    help="Render the richer lineage-pack schema instead of the compact default.",
)
@click.option(
    "--include-source",
    is_flag=True,
    help="Include source excerpts for leaf nodes when available.",
)
@click.option(
    "--source-budget",
    type=int,
    default=0,
    show_default=True,
    help="Maximum characters of leaf source excerpts to include.",
)
def knowledge_pack(
    selectors: tuple[str, ...],
    repo: str,
    output: str,
    output_format: str,
    depth: int,
    siblings: int,
    children: int,
    budget: int,
    detail: bool,
    include_source: bool,
    source_budget: int,
) -> None:
    """Render a deterministic lineage-specific knowledge pack."""
    if not selectors:
        raise click.UsageError("At least one context selector is required.")
    _, output_dir = _resolve_paths(repo, output)
    logger.info("Rendering lineage pack from %s", output_dir)
    click.echo(
        render_lineage_pack(
            output_dir,
            selectors,
            format=output_format,
            budget=budget,
            depth=depth,
            siblings=siblings,
            children=children,
            detail=detail,
            include_source=include_source,
            source_budget=source_budget,
        )
    )


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
    node = service.resolve_selector(node_id)
    click.echo(service.get_context(node.id).model_dump_json(indent=2))


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
    node = service.resolve_selector(node_id)
    _echo_refs(service.get_lineage(node.id, include_self=include_self))


@graph.command("children")
@click.argument("node_id")
@click.option("--repo", default=".", help="Repository root path.")
@click.option(
    "--output", default=".orbit/knowledge", help="Knowledge output directory."
)
def graph_children(node_id: str, repo: str, output: str) -> None:
    """Print immediate child nodes for a graph node."""
    service = _load_graph_context_service(repo, output)
    node = service.resolve_selector(node_id)
    _echo_refs(service.get_children(node.id))


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


def _build_component_names(target: str, output_dir: Path) -> list[str]:
    if target == "graph":
        return GRAPH_COMPONENT_NAMES
    if target == "knowledge":
        if (output_dir / "graph" / "refs" / "current.json").exists():
            return KNOWLEDGE_COMPONENT_NAMES
        return GRAPH_AND_KNOWLEDGE_COMPONENT_NAMES
    raise ValueError(f"Unsupported build target: {target}")


def _target_label(target: str) -> str:
    return "knowledge" if target == "knowledge" else target


def _load_graph_context_service(repo: str, output: str) -> GraphContextService:
    _, output_dir = _resolve_paths(repo, output)
    logger.debug("Loading graph context service from %s", output_dir)
    return GraphContextService.from_knowledge_dir(output_dir)


def _echo_refs(refs: list[NodeContextRef]) -> None:
    click.echo(json.dumps([ref.model_dump(mode="json") for ref in refs], indent=2))


if __name__ == "__main__":
    cli()
