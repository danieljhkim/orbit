from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, Sequence

from orbit_map.graph.extraction.base import leaf_location
from orbit_map.schemas import (
    FileSummaryV1,
    FileSymbolV1,
    LeafNode,
    WorkerHandoffPacket,
)
from orbit_map.schemas.graph.nodes import DirNode, FileNode
from orbit_map.service.graph_context import GraphContextService

LineagePackFormat = Literal["markdown", "json"]


@dataclass(slots=True)
class LineagePackRenderOptions:
    format: LineagePackFormat = "markdown"
    budget: int = 12_000
    depth: int = 2
    siblings: int = 2
    children: int = 4
    detail: bool = False
    include_source: bool = False
    source_budget: int = 0


def render_lineage_pack(
    knowledge_dir: Path | str,
    selectors: Sequence[str],
    *,
    format: LineagePackFormat = "markdown",
    budget: int = 12_000,
    depth: int = 2,
    siblings: int = 2,
    children: int = 4,
    detail: bool = False,
    include_source: bool = False,
    source_budget: int = 0,
) -> str:
    service = GraphContextService.from_knowledge_dir(knowledge_dir)
    options = LineagePackRenderOptions(
        format=format,
        budget=budget,
        depth=max(0, depth),
        siblings=max(0, siblings),
        children=max(0, children),
        detail=detail,
        include_source=include_source,
        source_budget=max(0, source_budget),
    )
    report = _build_verbose_report(service, selectors, options)
    if not detail:
        report = _compact_report(report)
    if format == "json":
        return json.dumps(report, indent=2, sort_keys=True)
    if detail:
        return _render_verbose_markdown(report, budget=budget)
    return _render_compact_markdown(report, budget=budget)


def render_lineage_pack_from_handoff(
    packet: WorkerHandoffPacket | dict[str, Any],
    **kwargs: Any,
) -> str:
    handoff = (
        packet
        if isinstance(packet, WorkerHandoffPacket)
        else WorkerHandoffPacket.model_validate(packet)
    )
    if not handoff.knowledge_dir:
        raise ValueError("Worker handoff packet requires knowledge_dir to render lineage pack")
    selectors = handoff.navigation_selectors()
    if not selectors:
        raise ValueError(
            "Worker handoff packet must include at least one lineage pack selector"
        )
    return render_lineage_pack(handoff.knowledge_dir, selectors, **kwargs)


def _build_verbose_report(
    service: GraphContextService,
    selectors: Sequence[str],
    options: LineagePackRenderOptions,
) -> dict[str, Any]:
    requested_nodes: list[tuple[str, Any]] = []
    seen_requested_ids: set[str] = set()
    context_nodes: dict[str, Any] = {}

    for selector in selectors:
        node = service.resolve_selector(selector)
        if node.id in seen_requested_ids:
            continue
        seen_requested_ids.add(node.id)
        requested_nodes.append((selector, node))

        focus_nodes = _bounded_lineage(service, node, options.depth)
        if node.node_type == "leaf":
            containing_file = service.navigator.get_containing_file(node.id)
            if containing_file is not None and containing_file.id not in {
                item.id for item in focus_nodes
            }:
                focus_nodes.insert(0, containing_file)

        for focus_node in focus_nodes:
            _add_context_node(service, context_nodes, focus_node, options)
            for sibling in _limited_siblings(service, focus_node, options.siblings):
                _add_context_node(service, context_nodes, sibling, options)
            for child in _limited_children(service, focus_node, options.children):
                _add_context_node(service, context_nodes, child, options)

    requested = [_selection_entry(service, selector, node, options) for selector, node in requested_nodes]
    nodes = [
        context_nodes[node_id]
        for node_id in sorted(
            context_nodes,
            key=lambda current_id: context_nodes[current_id]["selector"],
        )
    ]
    graph = service.graph

    return {
        "repo": {
            "root_selector": service.selector_for_node(service.navigator.get_root()),
            "requested_count": len(selectors),
            "selection_count": len(requested),
            "context_node_count": len(nodes),
            "dir_count": len(graph.dirs),
            "file_count": len(graph.files),
            "leaf_count": len(graph.leaves),
            "file_summary_count": sum(
                1
                for node in graph.files
                if node.source_blob_hash
                and node.source_blob_hash in service.file_summaries_by_hash
            ),
        },
        "overview": _generate_overview(service, requested_nodes),
        "options": {
            "depth": options.depth,
            "siblings": options.siblings,
            "children": options.children,
            "budget": options.budget,
            "include_source": options.include_source,
            "source_budget": options.source_budget,
        },
        "selections": requested,
        "nodes": nodes,
    }


def _generate_overview(
    service: GraphContextService, requested_nodes: list[tuple[str, Any]]
) -> str:
    if not requested_nodes:
        return ""

    if len(requested_nodes) == 1:
        selector, node = requested_nodes[0]
        if node.node_type == "file":
            context = service.get_file_context(node.id)
            summary = context.summary or "No summary available."
            parts = [f"File: `{selector}`. {summary}"]
            exports = list(context.exports)
            if exports:
                parts.append(f"Key exports: {', '.join(exports)}.")
            return " ".join(parts)

        if node.node_type == "dir":
            desc = node.description or "No description available."
            child_files = len(node.file_children)
            child_dirs = len(node.dir_children)
            return (
                f"Directory: `{selector}`. {desc} "
                f"Contains {child_files} files and {child_dirs} subdirectories."
            )

        if node.node_type == "leaf":
            sig = _leaf_signature(node)
            desc = node.description or "No description available."
            return f"Symbol: `{selector}`. Signature: `{sig}`. {desc}"

    # Multiple nodes selected
    shared_ancestor = _find_shared_ancestor(service, requested_nodes)
    ancestor_label = f"`{service.selector_for_node(shared_ancestor)}`" if shared_ancestor else "root"

    selections_desc = []
    for selector, node in requested_nodes[:5]:
        snippet = _node_overview_snippet(service, node)
        selections_desc.append(f"- `{selector}` ({node.node_type}): {snippet}")

    if len(requested_nodes) > 5:
        selections_desc.append(f"- ... and {len(requested_nodes) - 5} more.")

    header = f"Selection includes {len(requested_nodes)} nodes under {ancestor_label}:"
    return f"{header}\n" + "\n".join(selections_desc)


def _node_overview_snippet(service: GraphContextService, node: Any) -> str:
    if node.node_type == "file":
        summary = service.get_file_context(node.id).summary
        return _single_line(summary) if summary else "No summary available."
    if node.node_type == "dir":
        return _single_line(node.description) if node.description else "No description available."
    if node.node_type == "leaf":
        sig = _leaf_signature(node)
        desc = _single_line(node.description) if node.description else ""
        return f"`{sig}`. {desc}".strip()
    return ""


def _find_shared_ancestor(
    service: GraphContextService, requested_nodes: list[tuple[str, Any]]
) -> Any | None:
    if not requested_nodes:
        return None

    # Get lineages for all nodes
    lineages = [
        service.navigator.get_lineage(node.id, include_self=True)
        for _, node in requested_nodes
    ]

    # Find the deepest common node
    shared = None
    for i in range(min(len(lineage) for lineage in lineages)):
        current_nodes = [lineage[i] for lineage in lineages]
        if all(node.id == current_nodes[0].id for node in current_nodes):
            shared = current_nodes[0]
        else:
            break

    return shared


def _selection_entry(
    service: GraphContextService,
    requested_selector: str,
    node: Any,
    options: LineagePackRenderOptions,
) -> dict[str, Any]:
    lineage = [service.selector_for_node(item) for item in _bounded_lineage(service, node, options.depth)]
    if node.node_type == "leaf":
        containing_file = service.navigator.get_containing_file(node.id)
        if containing_file is not None:
            containing_selector = service.selector_for_node(containing_file)
            if containing_selector not in lineage:
                lineage.insert(0, containing_selector)

    entry: dict[str, Any] = {
        "selector": service.selector_for_node(node),
        "node_type": node.node_type,
        "lineage": lineage,
    }
    if requested_selector != entry["selector"]:
        entry["requested"] = requested_selector
    return entry


def _add_context_node(
    service: GraphContextService,
    context_nodes: dict[str, Any],
    node: Any,
    options: LineagePackRenderOptions,
) -> None:
    if node.id in context_nodes:
        return
    context_nodes[node.id] = _context_node_entry(service, node, options)


def _context_node_entry(
    service: GraphContextService,
    node: Any,
    options: LineagePackRenderOptions,
) -> dict[str, Any]:
    entry: dict[str, Any] = {
        "selector": service.selector_for_node(node),
        "node_type": node.node_type,
    }
    parent = service.navigator.get_parent(node.id)
    if parent is not None:
        entry["parent_selector"] = service.selector_for_node(parent)
    summary = _node_summary(service, node)
    if summary:
        entry["summary"] = summary

    siblings = _limited_siblings(service, node, options.siblings)
    if siblings:
        entry["siblings"] = [service.selector_for_node(item) for item in siblings]

    if node.node_type == "dir":
        child_dirs, child_files = _dir_children(service, node, options.children)
        entry["children"] = [
            service.selector_for_node(item) for item in [*child_dirs, *child_files]
        ]
        entry["details"] = {
            "child_dirs": [
                service.selector_for_node(item) for item in child_dirs
            ],
            "child_files": [
                service.selector_for_node(item) for item in child_files
            ],
        }
        return entry

    if node.node_type == "file":
        file_context = service.get_file_context(node.id)
        leaf_previews = [
            _leaf_preview_from_ref(service, node, ref)
            for ref in file_context.top_level_leaves[: options.children]
        ]
        entry["children"] = [preview["selector"] for preview in leaf_previews]
        entry["details"] = {
            "imports": list(file_context.imports),
            "exports": list(file_context.exports),
            "top_level_leaves": leaf_previews,
        }
        return entry

    if node.node_type == "leaf":
        child_leaves = _leaf_children(service, node, options.children)
        entry["children"] = [service.selector_for_node(item) for item in child_leaves]
        details: dict[str, Any] = {
            "kind": node.kind,
            "signature": _leaf_signature(node),
        }
        if node.description:
            details["description"] = node.description
        if options.include_source and options.source_budget > 0 and node.source:
            details["source"] = node.source[: options.source_budget]
        entry["details"] = details
        return entry

    raise ValueError(f"Unsupported graph node type: {type(node).__name__}")


def _bounded_lineage(
    service: GraphContextService, node: Any, depth: int
) -> list[Any]:
    lineage = service.navigator.get_lineage(node.id, include_self=True)
    if depth <= 0:
        return [lineage[-1]]
    return lineage[-(depth + 1) :]


def _limited_siblings(
    service: GraphContextService, node: Any, siblings: int
) -> list[Any]:
    if siblings <= 0:
        return []
    siblings_nodes = sorted(
        service.navigator.get_siblings(node.id),
        key=lambda item: item.location,
    )
    return siblings_nodes[:siblings]


def _limited_children(
    service: GraphContextService, node: Any, children: int
) -> list[Any]:
    if children <= 0:
        return []

    if node.node_type == "dir":
        child_dirs, child_files = _dir_children(service, node, children)
        return [*child_dirs, *child_files]
    if node.node_type == "file":
        file_context = service.get_file_context(node.id)
        return [
            service.navigator.get_node(item.id)
            for item in file_context.top_level_leaves[:children]
            if item.id in service.navigator.node_index
        ]
    if node.node_type == "leaf":
        return _leaf_children(service, node, children)
    return []


def _dir_children(
    service: GraphContextService, node: DirNode, children: int
) -> tuple[list[Any], list[Any]]:
    child_dirs = [
        service.navigator.get_dir(child_id)
        for child_id in node.dir_children[: children]
    ]
    child_files = [
        service.navigator.get_file(child_id)
        for child_id in node.file_children[: children]
    ]
    child_dirs.sort(key=lambda item: item.location)
    child_files.sort(key=lambda item: item.location)
    return child_dirs, child_files


def _leaf_children(
    service: GraphContextService, node: LeafNode, children: int
) -> list[LeafNode]:
    child_leaves = [
        service.navigator.get_leaf(child_id)
        for child_id in node.children[: children]
    ]
    child_leaves.sort(key=lambda item: item.location)
    return child_leaves


def _leaf_preview_from_ref(
    service: GraphContextService,
    file_node: FileNode,
    ref: Any,
) -> dict[str, Any]:
    node = service.navigator.node_index.get(ref.id)
    if isinstance(node, LeafNode):
        preview = {
            "selector": service.selector_for_node(node),
            "signature": _leaf_signature(node),
        }
        if node.description:
            preview["description"] = node.description
        return preview

    summary = _summary_for_file(service, file_node)
    symbol = _summary_symbol(summary, ref.name, ref.kind)
    preview = {
        "selector": _summary_selector(file_node.location, ref.name, ref.kind),
        "signature": symbol.signature if symbol is not None else ref.name,
    }
    if symbol is not None and symbol.description:
        preview["description"] = symbol.description
    elif ref.description:
        preview["description"] = ref.description
    return preview


def _node_summary(service: GraphContextService, node: Any) -> str:
    if isinstance(node, DirNode):
        return node.description
    if isinstance(node, FileNode):
        return service.get_file_context(node.id).summary
    if isinstance(node, LeafNode):
        return node.description
    return ""


def _summary_for_file(
    service: GraphContextService, file_node: FileNode
) -> FileSummaryV1 | None:
    if file_node.source_blob_hash is None:
        return None
    return service.file_summaries_by_hash.get(file_node.source_blob_hash)


def _summary_symbol(
    summary: FileSummaryV1 | None, name: str, kind: str | None
) -> FileSymbolV1 | None:
    if summary is None:
        return None
    for symbol in summary.symbols:
        if symbol.name == name and (kind is None or symbol.kind == kind):
            return symbol
    return None


def _leaf_signature(node: LeafNode) -> str:
    inputs = ", ".join(
        _signature_field(field.name, field.annotation) for field in node.input_signature
    )
    signature = f"{node.name}({inputs})" if inputs else f"{node.name}()"
    outputs = _render_outputs(node.output_signature)
    if outputs:
        signature += f" -> {outputs}"
    return signature


def _signature_field(name: str, annotation: str | None) -> str:
    return f"{name}: {annotation}" if annotation else name


def _render_outputs(fields: list[Any]) -> str:
    if not fields:
        return ""
    if len(fields) == 1:
        annotation = fields[0].annotation
        if annotation:
            return annotation
        return fields[0].name
    return ", ".join(_signature_field(field.name, field.annotation) for field in fields)


def _summary_selector(location: str, name: str, kind: str | None) -> str:
    suffix = f":{kind}" if kind else ""
    return f"leaf:{leaf_location(location, name)}{suffix}"


def _compact_report(verbose_report: dict[str, Any]) -> dict[str, Any]:
    compact_nodes: dict[str, dict[str, Any]] = {}
    for node in verbose_report["nodes"]:
        selector = node["selector"]
        compact_nodes[selector] = _compact_node(node)

    compact_selections = [_compact_selection(selection) for selection in verbose_report["selections"]]

    return {
        "repo": {
            "root_selector": verbose_report["repo"]["root_selector"],
            "requested_count": verbose_report["repo"]["requested_count"],
            "selection_count": verbose_report["repo"]["selection_count"],
            "context_node_count": verbose_report["repo"]["context_node_count"],
            "dir_count": verbose_report["repo"]["dir_count"],
            "file_count": verbose_report["repo"]["file_count"],
            "leaf_count": verbose_report["repo"]["leaf_count"],
            "file_summary_count": verbose_report["repo"]["file_summary_count"],
        },
        "overview": verbose_report.get("overview", ""),
        "options": {
            "depth": verbose_report["options"]["depth"],
            "siblings": verbose_report["options"]["siblings"],
            "children": verbose_report["options"]["children"],
            "budget": verbose_report["options"]["budget"],
            "detail": False,
            "include_source": verbose_report["options"]["include_source"],
            "source_budget": verbose_report["options"]["source_budget"],
        },
        "selections": compact_selections,
        "nodes": compact_nodes,
    }


def _compact_selection(selection: dict[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {
        "selector": selection["selector"],
        "lineage": selection["lineage"],
    }
    if selection.get("requested") is not None:
        compact["requested"] = selection["requested"]
    if selection.get("node_type") is not None:
        compact["type"] = selection["node_type"]
    return compact


def _compact_node(node: dict[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {"type": node["node_type"]}
    if node.get("parent_selector") is not None:
        compact["parent"] = node["parent_selector"]
    if node.get("summary"):
        compact["summary"] = node["summary"]
    if node.get("siblings"):
        compact["siblings"] = node["siblings"]

    details = node.get("details", {})
    if node["node_type"] == "dir":
        if node.get("children"):
            compact["children"] = node["children"]
        return compact

    if node["node_type"] == "file":
        if details.get("imports"):
            compact["imports"] = details["imports"]
        if details.get("exports"):
            compact["exports"] = details["exports"]
        top_level_leaves = details.get("top_level_leaves", [])
        if top_level_leaves:
            compact["leaves"] = [
                _compact_leaf_preview(preview) for preview in top_level_leaves
            ]
        return compact

    if node["node_type"] == "leaf":
        if node.get("children"):
            compact["children"] = node["children"]
        if details.get("kind"):
            compact["kind"] = details["kind"]
        if details.get("signature"):
            compact["signature"] = details["signature"]
        if details.get("description"):
            compact["description"] = details["description"]
        if details.get("source"):
            compact["source"] = details["source"]
        return compact

    return compact


def _compact_leaf_preview(preview: dict[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {
        "selector": preview["selector"],
        "signature": preview["signature"],
    }
    if preview.get("description"):
        compact["description"] = preview["description"]
    return compact


def _render_compact_markdown(report: dict[str, Any], budget: int) -> str:
    writer = _MarkdownWriter(budget=budget)
    repo = report["repo"]
    selections = report["selections"]
    nodes = report["nodes"]

    writer.line("# Lineage Pack")
    writer.line()
    writer.line("## Repo")
    writer.line(f"- root: `{repo['root_selector']}`")
    writer.line(f"- selections: {repo['selection_count']}")
    writer.line(f"- context nodes: {repo['context_node_count']}")
    writer.line(f"- files: {repo['file_count']}")
    writer.line(f"- leaves: {repo['leaf_count']}")
    writer.line()

    if report.get("overview"):
        writer.line("## Overview")
        for overview_line in report["overview"].splitlines():
            if not writer.line(overview_line):
                break
        writer.line()

    writer.line("## Selections")
    for selection in selections:
        if not writer.line(f"- `{selection['selector']}`"):
            break
        if selection.get("requested") and selection["requested"] != selection["selector"]:
            requested = selection["requested"]
            if isinstance(requested, list):
                rendered = ", ".join(f"`{item}`" for item in requested)
                writer.line(f"  - requested: {rendered}")
            else:
                writer.line(f"  - requested: `{requested}`")
        writer.line(f"  - lineage: {' -> '.join(f'`{item}`' for item in selection['lineage'])}")
        if selection.get("type"):
            writer.line(f"  - type: `{selection['type']}`")
        writer.line()
        if writer.truncated:
            break

    writer.line("## Context")
    for selector in sorted(nodes):
        node = nodes[selector]
        if not writer.line(f"### `{selector}`"):
            break
        if node.get("type"):
            writer.line(f"- type: `{node['type']}`")
        if node.get("parent"):
            writer.line(f"- parent: `{node['parent']}`")
        if node.get("summary"):
            writer.line(f"- summary: {_single_line(node['summary'])}")
        if node.get("siblings"):
            writer.line(
                f"- siblings: {', '.join(f'`{item}`' for item in node['siblings'])}"
            )
        if node.get("children"):
            writer.line(
                f"- children: {', '.join(f'`{item}`' for item in node['children'])}"
            )

        if node["type"] == "file":
            if node.get("imports"):
                writer.line(f"- imports: {', '.join(node['imports']) or '(none)'}")
            if node.get("exports"):
                writer.line(f"- exports: {', '.join(node['exports']) or '(none)'}")
            if node.get("leaves"):
                writer.line("- leaves:")
                for leaf in node["leaves"]:
                    line = f"  - `{leaf['signature']}` (`{leaf['selector']}`)"
                    if leaf.get("description"):
                        line += f" - {_single_line(leaf['description'])}"
                    if not writer.line(line):
                        break
        elif node["type"] == "leaf":
            if node.get("kind"):
                writer.line(f"- kind: `{node['kind']}`")
            if node.get("signature"):
                writer.line(f"- signature: `{node['signature']}`")
            if node.get("description"):
                writer.line(f"- description: {_single_line(node['description'])}")
            if node.get("source"):
                writer.line("- source excerpt:")
                for source_line in node["source"].splitlines():
                    if not writer.line(f"  - `{source_line}`"):
                        break
        writer.line()
        if writer.truncated:
            break

    if writer.truncated:
        writer.line("_output truncated to respect the requested budget._")

    return writer.text()


def _render_verbose_markdown(report: dict[str, Any], budget: int) -> str:
    writer = _MarkdownWriter(budget=budget)
    repo = report["repo"]
    options = report["options"]
    selections = report["selections"]
    nodes = report["nodes"]

    writer.line("# Lineage Pack")
    writer.line()
    writer.line("## Repo Stats")
    writer.line(f"- root: `{repo['root_selector']}`")
    writer.line(f"- requested selectors: {repo['requested_count']}")
    writer.line(f"- selections: {repo['selection_count']}")
    writer.line(f"- context nodes: {repo['context_node_count']}")
    writer.line(f"- directories: {repo['dir_count']}")
    writer.line(f"- files: {repo['file_count']}")
    writer.line(f"- leaves: {repo['leaf_count']}")
    writer.line(f"- file summaries: {repo['file_summary_count']}")
    writer.line(
        f"- traversal bounds: depth={options['depth']}, siblings={options['siblings']}, children={options['children']}"
    )
    writer.line()

    if report.get("overview"):
        writer.line("## Overview")
        for overview_line in report["overview"].splitlines():
            if not writer.line(overview_line):
                break
        writer.line()

    writer.line("## Selections")
    for selection in selections:
        header = f"### `{selection['selector']}`"
        if selection.get("requested"):
            header += f" from `{selection['requested']}`"
        if not writer.line(header):
            break
        writer.line(f"- type: `{selection['node_type']}`")
        writer.line(f"- lineage: {' -> '.join(f'`{item}`' for item in selection['lineage'])}")
        writer.line()
        if writer.truncated:
            break

    writer.line("## Context Nodes")
    for node in nodes:
        if not writer.line(f"### `{node['selector']}`"):
            break
        writer.line(f"- type: `{node['node_type']}`")
        if node.get("parent_selector"):
            writer.line(f"- parent: `{node['parent_selector']}`")
        if node.get("summary"):
            writer.line(f"- summary: {_single_line(node['summary'])}")
        if node.get("siblings"):
            writer.line(
                f"- siblings: {', '.join(f'`{item}`' for item in node['siblings'])}"
            )
        if node.get("children"):
            writer.line(
                f"- children: {', '.join(f'`{item}`' for item in node['children'])}"
            )

        details = node.get("details", {})
        if node["node_type"] == "dir":
            child_dirs = details.get("child_dirs", [])
            child_files = details.get("child_files", [])
            if child_dirs:
                writer.line(
                    f"- child dirs: {', '.join(f'`{item}`' for item in child_dirs)}"
                )
            if child_files:
                writer.line(
                    f"- child files: {', '.join(f'`{item}`' for item in child_files)}"
                )
        elif node["node_type"] == "file":
            if details.get("imports"):
                writer.line(
                    f"- imports: {', '.join(details['imports']) or '(none)'}"
                )
            if details.get("exports"):
                writer.line(
                    f"- exports: {', '.join(details['exports']) or '(none)'}"
                )
            top_level_leaves = details.get("top_level_leaves", [])
            if top_level_leaves:
                writer.line("- top-level leaves:")
                for leaf in top_level_leaves:
                    line = f"  - `{leaf['signature']}` (`{leaf['selector']}`)"
                    if leaf.get("description"):
                        line += f" - {_single_line(leaf['description'])}"
                    if not writer.line(line):
                        break
        elif node["node_type"] == "leaf":
            if details.get("kind"):
                writer.line(f"- kind: `{details['kind']}`")
            if details.get("signature"):
                writer.line(f"- signature: `{details['signature']}`")
            if details.get("description"):
                writer.line(f"- description: {_single_line(details['description'])}")
            if details.get("source"):
                writer.line("- source excerpt:")
                for source_line in details["source"].splitlines():
                    if not writer.line(f"  - `{source_line}`"):
                        break
        writer.line()
        if writer.truncated:
            break

    if writer.truncated:
        writer.line("_output truncated to respect the requested budget._")

    return writer.text()


def _single_line(text: str | None) -> str:
    if not text:
        return "(none)"
    return " ".join(text.split())


class _MarkdownWriter:
    def __init__(self, budget: int):
        self.budget = budget
        self._parts: list[str] = []
        self._length = 0
        self.truncated = False

    def line(self, text: str = "") -> bool:
        if self.truncated:
            return False
        piece = f"{text}\n"
        if self._length + len(piece) > self.budget:
            self.truncated = True
            return False
        self._parts.append(piece)
        self._length += len(piece)
        return True

    def text(self) -> str:
        return "".join(self._parts).rstrip() + "\n"
