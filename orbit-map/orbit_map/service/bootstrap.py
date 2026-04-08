from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

from orbit_map.graph.extraction.base import leaf_location
from orbit_map.schemas import FileSummaryV1, FileSymbolV1, LeafNode
from orbit_map.schemas.graph.nodes import DirNode, FileNode
from orbit_map.service.graph_context import GraphContextService

BootstrapFormat = Literal["markdown", "json"]


@dataclass(slots=True)
class BootstrapRenderOptions:
    format: BootstrapFormat = "markdown"
    budget: int = 12_000
    include_source: bool = False
    source_budget: int = 0


def render_knowledge_bootstrap(
    knowledge_dir: Path | str,
    *,
    format: BootstrapFormat = "markdown",
    budget: int = 12_000,
    include_source: bool = False,
    source_budget: int = 0,
) -> str:
    service = GraphContextService.from_knowledge_dir(knowledge_dir)
    options = BootstrapRenderOptions(
        format=format,
        budget=budget,
        include_source=include_source,
        source_budget=source_budget,
    )
    report = _build_report(service, options)
    if format == "json":
        return json.dumps(report, indent=2, sort_keys=True)
    return _render_markdown(report, budget=budget)


def _build_report(
    service: GraphContextService, options: BootstrapRenderOptions
) -> dict[str, Any]:
    graph = service.graph
    tree = _build_tree(service, graph.root_dir_id)
    files = [
        _build_file_report(service, file_node, options)
        for file_node in sorted(graph.files, key=lambda node: node.location)
    ]
    summary_count = sum(
        1
        for node in graph.files
        if node.source_blob_hash
        and node.source_blob_hash in service.file_summaries_by_hash
    )

    return {
        "repo": {
            "root_selector": _selector_for_dir(service.navigator.get_root()),
            "dir_count": len(graph.dirs),
            "file_count": len(graph.files),
            "leaf_count": len(graph.leaves),
            "file_summary_count": summary_count,
        },
        "tree": tree,
        "files": files,
    }


def _build_tree(service: GraphContextService, dir_id: str) -> dict[str, Any]:
    node = service.navigator.get_dir(dir_id)
    child_dirs = sorted(
        (service.navigator.get_dir(child_id) for child_id in node.dir_children),
        key=lambda child: child.location,
    )
    child_files = sorted(
        (service.navigator.get_file(child_id) for child_id in node.file_children),
        key=lambda child: child.location,
    )
    return {
        "selector": _selector_for_dir(node),
        "children": [
            _build_tree(service, child_dir.id) for child_dir in child_dirs
        ]
        + [_build_tree_file_stub(file_node) for file_node in child_files],
    }


def _build_tree_file_stub(file_node: FileNode) -> dict[str, Any]:
    return {
        "selector": _selector_for_file(file_node),
    }


def _build_file_report(
    service: GraphContextService,
    file_node: FileNode,
    options: BootstrapRenderOptions,
) -> dict[str, Any]:
    context = service.get_file_context(file_node.id)
    summary = _summary_for_file(service, file_node)
    leaves = [
        _build_leaf_report(service, file_node, ref, summary, options)
        for ref in context.top_level_leaves
    ]

    return {
        "selector": _selector_for_file(file_node),
        "lineage": [
            _selector_for_dir(node) for node in _dir_lineage(service, file_node)
        ]
        + [_selector_for_file(file_node)],
        "summary": context.summary,
        "imports": list(context.imports),
        "exports": list(context.exports),
        "top_level_leaves": leaves,
    }


def _build_leaf_report(
    service: GraphContextService,
    file_node: FileNode,
    ref: Any,
    summary: FileSummaryV1 | None,
    options: BootstrapRenderOptions,
) -> dict[str, Any]:
    node = service.navigator.node_index.get(ref.id)
    if node is not None and node.node_type == "leaf":
        signature = _leaf_signature(node)
        description = node.description
        source = node.source
        selector = _selector_for_leaf(node)
    else:
        symbol = _summary_symbol(summary, ref.name, ref.kind)
        signature = symbol.signature if symbol is not None else ref.name
        description = symbol.description if symbol is not None else ref.description
        source = ""
        selector = _summary_selector(file_node.location, ref.name, ref.kind)

    leaf_report = {
        "selector": selector,
        "signature": signature,
    }
    if description:
        leaf_report["description"] = description
    if options.include_source and options.source_budget > 0 and source:
        leaf_report["source"] = source[: options.source_budget]
    return leaf_report


def _render_markdown(report: dict[str, Any], budget: int) -> str:
    writer = _MarkdownWriter(budget=budget)
    repo = report["repo"]
    tree = report["tree"]
    files = report["files"]

    writer.line("# Codebase Bootstrap")
    writer.line()
    writer.line("## Repo Stats")
    writer.line(f"- root: `{repo['root_selector']}`")
    writer.line(f"- directories: {repo['dir_count']}")
    writer.line(f"- files: {repo['file_count']}")
    writer.line(f"- leaves: {repo['leaf_count']}")
    writer.line(f"- file summaries: {repo['file_summary_count']}")
    writer.line()
    writer.line("## Directory Tree")
    _render_tree(writer, tree, indent=0)
    writer.line()
    writer.line("## Files")
    for file_report in files:
        if not writer.line(f"### `{file_report['selector']}`"):
            break
        writer.line(f"- lineage: {' -> '.join(f'`{item}`' for item in file_report['lineage'])}")
        writer.line(f"- summary: {_single_line(file_report['summary'])}")
        writer.line(f"- imports: {', '.join(file_report['imports']) or '(none)'}")
        writer.line(f"- exports: {', '.join(file_report['exports']) or '(none)'}")
        if file_report["top_level_leaves"]:
            writer.line("- top-level leaves:")
            for leaf_report in file_report["top_level_leaves"]:
                line = (
                    f"  - `{leaf_report['signature']}` "
                    f"(`{leaf_report['selector']}`)"
                )
                if leaf_report.get("description"):
                    line += f" - {_single_line(leaf_report['description'])}"
                if not writer.line(line):
                    break
                if leaf_report.get("source"):
                    if not writer.line("    - source excerpt:"):
                        break
                    for source_line in leaf_report["source"].splitlines():
                        if not writer.line(f"      - `{source_line}`"):
                            break
        else:
            writer.line("- top-level leaves: (none)")
        writer.line()
        if writer.truncated:
            break

    if writer.truncated:
        writer.line("_output truncated to respect the requested budget._")

    return writer.text()


def _render_tree(writer: "_MarkdownWriter", tree: dict[str, Any], indent: int) -> None:
    prefix = "  " * indent
    writer.line(f"{prefix}- `{tree['selector']}`")
    for child in tree["children"]:
        if "children" in child:
            _render_tree(writer, child, indent + 1)
        else:
            writer.line(f"{prefix}  - `{child['selector']}`")


def _dir_lineage(service: GraphContextService, file_node: FileNode) -> list[DirNode]:
    dirs: list[DirNode] = []
    current = service.navigator.get_parent(file_node.id)
    while isinstance(current, DirNode):
        dirs.append(current)
        current = service.navigator.get_parent(current.id)
    dirs.reverse()
    return dirs


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


def _single_line(text: str | None) -> str:
    if not text:
        return "(none)"
    return " ".join(text.split())


def _selector_for_dir(node: DirNode) -> str:
    return f"dir:{node.location}"


def _selector_for_file(node: FileNode) -> str:
    return f"file:{node.location}"


def _selector_for_leaf(node: LeafNode) -> str:
    return f"leaf:{node.location}:{node.kind}"


def _summary_selector(location: str, name: str, kind: str | None) -> str:
    suffix = f":{kind}" if kind else ""
    return f"leaf:{leaf_location(location, name)}{suffix}"


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
