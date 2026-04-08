from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence

from orbit_map.schemas import FileSummaryV1, FileSymbolV1, HandoffConstraint, HandoffRisk
from orbit_map.schemas.graph.contexts import NodeContextRef
from orbit_map.schemas.graph.handoff import HandoffNodeRef, WorkerHandoffPacket
from orbit_map.schemas.graph.nodes import DirNode, FileNode, LeafNode
from orbit_map.service.graph_context import GraphContextService


@dataclass(slots=True)
class LineageBriefOptions:
    task_id: str
    task_intent: str
    task_title: str = ""
    root_selectors: Sequence[str] = ()
    target_selectors: Sequence[str] = ()
    write_selectors: Sequence[str] = ()
    read_only_selectors: Sequence[str] = ()
    risks: Sequence[HandoffRisk | dict[str, Any] | str] = ()
    constraints: Sequence[HandoffConstraint | dict[str, Any] | str] = ()
    budget: int = 12_000
    include_source: bool = True
    source_budget: int = 0


def build_lineage_brief(
    knowledge_dir: Path | str,
    *,
    task_id: str,
    task_intent: str,
    task_title: str = "",
    root_selectors: Sequence[str],
    target_selectors: Sequence[str],
    write_selectors: Sequence[str] = (),
    read_only_selectors: Sequence[str] = (),
    risks: Sequence[HandoffRisk | dict[str, Any] | str] = (),
    constraints: Sequence[HandoffConstraint | dict[str, Any] | str] = (),
    budget: int = 12_000,
    include_source: bool = True,
    source_budget: int = 0,
) -> str:
    if not root_selectors:
        raise ValueError("Lineage brief requires at least one root selector")
    if not target_selectors:
        raise ValueError("Lineage brief requires at least one target selector")

    options = LineageBriefOptions(
        task_id=task_id,
        task_title=task_title,
        task_intent=task_intent,
        root_selectors=tuple(root_selectors),
        target_selectors=tuple(target_selectors),
        write_selectors=tuple(write_selectors) if write_selectors else tuple(target_selectors),
        read_only_selectors=tuple(read_only_selectors),
        risks=tuple(risks),
        constraints=tuple(constraints),
        budget=max(1, budget),
        include_source=include_source,
        source_budget=max(0, source_budget),
    )

    service = GraphContextService.from_knowledge_dir(knowledge_dir)
    packet = service.build_handoff_packet(
        task_id=options.task_id,
        task_title=options.task_title,
        task_intent=options.task_intent,
        root_selectors=options.root_selectors,
        target_selectors=options.target_selectors,
        write_selectors=options.write_selectors,
        read_only_selectors=options.read_only_selectors,
        risks=[_normalize_risk(item) for item in options.risks],
        constraints=[_normalize_constraint(item) for item in options.constraints],
        knowledge_dir=str(Path(knowledge_dir)),
    )

    target_details = [
        _build_target_detail(service, ref, options) for ref in packet.target_nodes
    ]
    target_leaf_ids = {
        leaf["id"]
        for detail in target_details
        for leaf in detail["target_leaves"]
        if leaf.get("id")
    }
    context_nodes = _build_context_nodes(service, packet, target_leaf_ids)
    return _render_lineage_brief(packet, target_details, context_nodes, options)


def _render_lineage_brief(
    packet: WorkerHandoffPacket,
    target_details: list[dict[str, Any]],
    context_nodes: list[dict[str, Any]],
    options: LineageBriefOptions,
) -> str:
    writer = _MarkdownWriter(budget=options.budget)

    writer.line("# Lineage Brief")
    writer.line()
    writer.line("## Task")
    writer.line(f"- task id: `{packet.task_id}`")
    writer.line(f"- task title: {packet.task_title or '(none)'}")
    writer.line(f"- task intent: {packet.task_intent}")
    writer.line()
    writer.line("## Assigned Lineage")
    _write_node_section(writer, "Roots", packet.root_nodes)
    _write_node_section(writer, "Targets", packet.target_nodes)
    _write_lineage_paths(writer, target_details)
    if not writer.truncated:
        writer.line("## Target Nodes")
        if target_details:
            for detail in target_details:
                if not _write_target_detail(writer, detail):
                    break
        else:
            writer.line("- (none)")
        writer.line()
    if not writer.truncated:
        writer.line("## Context Summaries")
        if context_nodes:
            for entry in context_nodes:
                summary = entry["summary"]
                roles = ", ".join(entry["roles"])
                line = (
                    f"- `{entry['selector']}` | roles: {roles} | type: {entry['node_type']}"
                )
                if summary:
                    line += f" | {summary}"
                if not writer.line(line):
                    break
        else:
            writer.line("- (none)")
        writer.line()
    if not writer.truncated:
        writer.line("## Edit Boundaries")
        _write_node_section(writer, "Writable Scope", packet.write_nodes)
        _write_node_section(writer, "Read-Only Scope", packet.read_only_nodes)
    if not writer.truncated:
        writer.line("## Risks")
        if packet.risks:
            for risk in packet.risks:
                if not writer.line(risk.to_markdown_line()):
                    break
        else:
            writer.line("- (none)")
        writer.line()
    if not writer.truncated:
        writer.line("## Constraints")
        if packet.constraints:
            for constraint in packet.constraints:
                if not writer.line(constraint.to_markdown_line()):
                    break
        else:
            writer.line("- (none)")
        writer.line()
    if not writer.truncated:
        writer.line("## Navigation")
        writer.line(f"- knowledge dir: `{packet.knowledge_dir or '(none)'}`")
        writer.line(
            "- graph selectors: "
            + (
                ", ".join(f"`{selector}`" for selector in packet.navigation_selectors())
                if packet.navigation_selectors()
                else "(none)"
            )
        )
        if packet.target_nodes:
            writer.line(
                "- inspect first target: "
                f"`orbit-map graph context {packet.target_nodes[0].selector}`"
            )
            writer.line(
                "- trace first target lineage: "
                f"`orbit-map graph lineage {packet.target_nodes[0].selector} --include-self`"
            )
        if packet.navigation_selectors():
            selector_args = " ".join(packet.navigation_selectors())
            writer.line(f"- render lineage pack: `orbit-map knowledge pack {selector_args}`")

    if writer.truncated:
        writer.note("_output truncated to respect the requested budget._")

    return writer.text()


def _write_lineage_paths(
    writer: "_MarkdownWriter", target_details: list[dict[str, Any]]
) -> None:
    if not writer.line("### Lineage Paths"):
        return
    if target_details:
        for detail in target_details:
            if not writer.line(
                f"- `{detail['selector']}`: "
                + " -> ".join(f"`{selector}`" for selector in detail["lineage"])
            ):
                break
    else:
        writer.line("- (none)")
    writer.line()


def _write_target_detail(writer: "_MarkdownWriter", detail: dict[str, Any]) -> bool:
    if not writer.line(f"### `{detail['selector']}`"):
        return False
    writer.line(f"- type: {detail['node_type']}")
    writer.line(f"- writable: {'yes' if detail['is_writable'] else 'no'}")
    writer.line(
        "- lineage: " + " -> ".join(f"`{selector}`" for selector in detail["lineage"])
    )
    if detail["summary"]:
        writer.line(f"- summary: {detail['summary']}")
    if detail["target_leaves"]:
        writer.line("- target leaves:")
        for leaf in detail["target_leaves"]:
            line = f"  - `{leaf['signature']}` (`{leaf['selector']}`)"
            if leaf.get("file_selector"):
                line += f" | file: `{leaf['file_selector']}`"
            if leaf.get("description"):
                line += f" | {leaf['description']}"
            if not writer.line(line):
                return False
            source = leaf.get("source")
            if source:
                if not writer.line("    ```"):
                    return False
                for source_line in source.splitlines():
                    if not writer.line(f"    {source_line}"):
                        return False
                if not writer.line("    ```"):
                    return False
    else:
        writer.line("- target leaves: (none)")
    writer.line()
    return not writer.truncated


def _write_node_section(
    writer: "_MarkdownWriter", heading: str, nodes: list[HandoffNodeRef]
) -> None:
    if not writer.line(f"### {heading}"):
        return
    if nodes:
        for node in nodes:
            if not writer.line(node.to_markdown_line()):
                break
    else:
        writer.line("- (none)")
    writer.line()


def _build_target_detail(
    service: GraphContextService,
    ref: HandoffNodeRef,
    options: LineageBriefOptions,
) -> dict[str, Any]:
    node = service.get_node(ref.id)
    return {
        "selector": ref.selector,
        "node_type": ref.node_type,
        "summary": _single_line(_node_summary(service, node)),
        "lineage": [
            service.selector_for_node(item)
            for item in service.navigator.get_lineage(node.id, include_self=True)
        ],
        "is_writable": _is_node_covered(service, node, options.write_selectors),
        "target_leaves": _expand_target_leaves(service, node, options),
    }


def _build_context_nodes(
    service: GraphContextService,
    packet: WorkerHandoffPacket,
    target_leaf_ids: set[str],
) -> list[dict[str, Any]]:
    target_ids = {ref.id for ref in packet.target_nodes}
    tagged_ids: dict[str, set[str]] = {}

    for ref in packet.root_nodes:
        _add_role(tagged_ids, ref.id, "root")
    for ref in packet.write_nodes:
        _add_role(tagged_ids, ref.id, "write")
    for ref in packet.read_only_nodes:
        _add_role(tagged_ids, ref.id, "read-only")

    for ref in packet.target_nodes:
        for ancestor in service.navigator.get_lineage(ref.id):
            _add_role(tagged_ids, ancestor.id, "lineage")
        for sibling in sorted(
            service.navigator.get_siblings(ref.id), key=lambda item: item.location
        )[:2]:
            _add_role(tagged_ids, sibling.id, "sibling")

    entries: list[dict[str, Any]] = []
    for node_id in sorted(
        tagged_ids,
        key=lambda current_id: service.selector_for_node(service.get_node(current_id)),
    ):
        if node_id in target_ids or node_id in target_leaf_ids:
            continue
        node = service.get_node(node_id)
        entries.append(
            {
                "selector": service.selector_for_node(node),
                "node_type": node.node_type,
                "roles": sorted(tagged_ids[node_id]),
                "summary": _single_line(_node_summary(service, node)),
            }
        )
    return entries


def _expand_target_leaves(
    service: GraphContextService,
    node: DirNode | FileNode | LeafNode,
    options: LineageBriefOptions,
) -> list[dict[str, Any]]:
    if isinstance(node, LeafNode):
        return [_leaf_detail_from_node(service, node, options)]
    if isinstance(node, FileNode):
        return _leaf_details_from_file(service, node, options)

    details: list[dict[str, Any]] = []
    for file_node in _descendant_files(service, node):
        details.extend(_leaf_details_from_file(service, file_node, options))
    return details


def _leaf_details_from_file(
    service: GraphContextService,
    file_node: FileNode,
    options: LineageBriefOptions,
) -> list[dict[str, Any]]:
    context = service.get_file_context(file_node.id)
    summary = _summary_for_file(service, file_node)
    return [
        _leaf_detail_from_ref(service, file_node, ref, summary, options)
        for ref in context.top_level_leaves
    ]


def _leaf_detail_from_node(
    service: GraphContextService,
    node: LeafNode,
    options: LineageBriefOptions,
) -> dict[str, Any]:
    containing_file = service.navigator.get_containing_file(node.id)
    return {
        "id": node.id,
        "selector": service.selector_for_node(node),
        "signature": _leaf_signature(node),
        "description": _single_line(node.description),
        "file_selector": (
            service.selector_for_node(containing_file) if containing_file is not None else ""
        ),
        "source": _render_leaf_source(node.source, options),
    }


def _leaf_detail_from_ref(
    service: GraphContextService,
    file_node: FileNode,
    ref: NodeContextRef,
    summary: FileSummaryV1 | None,
    options: LineageBriefOptions,
) -> dict[str, Any]:
    node = service.navigator.node_index.get(ref.id)
    if isinstance(node, LeafNode):
        return _leaf_detail_from_node(service, node, options)

    symbol = _summary_symbol(summary, ref.name, ref.kind)
    return {
        "id": ref.id,
        "selector": _selector_for_ref(ref),
        "signature": symbol.signature if symbol is not None else ref.name,
        "description": _single_line(
            symbol.description if symbol is not None else ref.description
        ),
        "file_selector": service.selector_for_node(file_node),
    }


def _render_leaf_source(source: str, options: LineageBriefOptions) -> str:
    if not options.include_source or not source:
        return ""
    if options.source_budget > 0:
        return source[: options.source_budget]
    return source


def _descendant_files(
    service: GraphContextService, dir_node: DirNode
) -> list[FileNode]:
    files: list[FileNode] = []
    queue = [dir_node]

    while queue:
        current = queue.pop(0)
        child_dirs = [
            service.navigator.get_dir(child_id) for child_id in current.dir_children
        ]
        child_files = [
            service.navigator.get_file(child_id) for child_id in current.file_children
        ]
        child_dirs.sort(key=lambda item: item.location)
        child_files.sort(key=lambda item: item.location)
        queue.extend(child_dirs)
        files.extend(child_files)

    return files


def _normalize_risk(item: HandoffRisk | dict[str, Any] | str) -> dict[str, Any]:
    if isinstance(item, HandoffRisk):
        return item.model_dump(mode="json")
    if isinstance(item, str):
        return {"description": item}
    return item


def _normalize_constraint(
    item: HandoffConstraint | dict[str, Any] | str,
) -> dict[str, Any]:
    if isinstance(item, HandoffConstraint):
        return item.model_dump(mode="json")
    if isinstance(item, str):
        return {"description": item}
    return item


def _add_role(tagged_ids: dict[str, set[str]], node_id: str, role: str) -> None:
    tagged_ids.setdefault(node_id, set()).add(role)


def _is_node_covered(
    service: GraphContextService, node: DirNode | FileNode | LeafNode, selectors: Sequence[str]
) -> bool:
    for selector in selectors:
        scope_node = service.resolve_selector(selector)
        current: DirNode | FileNode | LeafNode | None = node
        while current is not None:
            if current.id == scope_node.id:
                return True
            parent = service.navigator.get_parent(current.id)
            current = parent if parent is not None else None
    return False


def _node_summary(service: GraphContextService, node: DirNode | FileNode | LeafNode) -> str:
    if isinstance(node, DirNode):
        return node.description
    if isinstance(node, FileNode):
        return service.get_file_context(node.id).summary
    return node.description or _leaf_signature(node)


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


def _selector_for_ref(ref: NodeContextRef) -> str:
    if ref.node_type == "dir":
        return f"dir:{ref.location}"
    if ref.node_type == "file":
        return f"file:{ref.location}"
    return f"leaf:{ref.location}:{ref.kind}"


def _single_line(text: str | None) -> str:
    if not text:
        return ""
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

    def note(self, text: str) -> None:
        piece = f"{text}\n"
        if self._length + len(piece) <= self.budget:
            self._parts.append(piece)
            self._length += len(piece)

    def text(self) -> str:
        if not self._parts:
            return ""
        return "".join(self._parts).rstrip() + "\n"
