from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any, Iterable, Sequence

from orbit_map.graph.extraction.base import node_id
from orbit_map.graph.store import GraphObjectStore
from orbit_map.schemas import (
    CodebaseGraphV1,
    DirContext,
    FileContext,
    FileSummaryV1,
    FileSymbolV1,
    HandoffConstraint,
    HandoffNodeRef,
    HandoffRisk,
    LeafContext,
    NodeContextRef,
    WorkerHandoffPacket,
)
from orbit_map.schemas.graph.contexts import NodeType
from orbit_map.schemas.graph.navigation import GraphNavigator, GraphNode
from orbit_map.schemas.graph.nodes import FileNode, LeafKind

logger = logging.getLogger(__name__)


class GraphContextService:
    def __init__(
        self,
        graph: CodebaseGraphV1,
        file_summaries_by_hash: dict[str, FileSummaryV1] | None = None,
    ):
        self.graph = graph
        self.navigator = GraphNavigator(graph)
        self.file_summaries_by_hash = file_summaries_by_hash or {}

    @classmethod
    def from_graph_dir(cls, graph_dir: Path | str) -> GraphContextService:
        return cls(GraphObjectStore(graph_dir).read_graph())

    @classmethod
    def from_knowledge_dir(cls, knowledge_dir: Path | str) -> GraphContextService:
        path = Path(knowledge_dir)
        graph = GraphObjectStore(path / "graph").read_graph()
        return cls(graph, _load_file_summaries(path / "files"))

    def get_node(self, node_id: str) -> GraphNode:
        return self.navigator.get_node(node_id)

    def resolve_selector(self, selector: str) -> GraphNode:
        normalized = selector.strip()
        if not normalized:
            raise ValueError("Empty node selector")

        if normalized in self.navigator.node_index:
            return self.get_node(normalized)

        node_type, location, kind = _parse_selector(normalized)
        if node_type == "dir":
            for node in self.graph.dirs:
                if node.location == location:
                    return node
            raise ValueError(f"Unknown dir selector: {selector}")
        if node_type == "file":
            for node in self.graph.files:
                if node.location == location:
                    return node
            raise ValueError(f"Unknown file selector: {selector}")

        for node in self.graph.leaves:
            if node.location == location and node.kind == kind:
                return node
        raise ValueError(f"Unknown leaf selector: {selector}")

    def selector_for_node(self, node: GraphNode) -> str:
        if node.node_type == "dir":
            return f"dir:{node.location}"
        if node.node_type == "file":
            return f"file:{node.location}"
        return f"leaf:{node.location}:{node.kind}"

    def get_parent(self, node_id: str) -> NodeContextRef | None:
        parent = self.navigator.get_parent(node_id)
        if parent is None:
            return None
        return self.navigator.to_ref(parent)

    def get_children(self, node_id: str) -> list[NodeContextRef]:
        return self.navigator.to_refs(self.navigator.get_children(node_id))

    def get_siblings(self, node_id: str) -> list[NodeContextRef]:
        return self.navigator.to_refs(self.navigator.get_siblings(node_id))

    def get_lineage(
        self, node_id: str, include_self: bool = False
    ) -> list[NodeContextRef]:
        return self.navigator.to_refs(
            self.navigator.get_lineage(node_id, include_self=include_self)
        )

    def get_dir_context(self, dir_id: str) -> DirContext:
        return self.navigator.get_dir_context(dir_id)

    def get_file_context(self, file_id: str) -> FileContext:
        context = self.navigator.get_file_context(file_id)
        summary = self._summary_for_file(context.node)
        if summary is None:
            return context

        top_level_leaves = context.top_level_leaves
        if not top_level_leaves:
            top_level_leaves = [
                _symbol_to_ref(context.node, summary.path, symbol)
                for symbol in summary.symbols
            ]

        return context.model_copy(
            update={
                "imports": summary.imports or context.imports,
                "exports": summary.exports or context.exports,
                "summary": summary.summary or context.summary,
                "top_level_leaves": top_level_leaves,
            }
        )

    def get_leaf_context(self, leaf_id: str) -> LeafContext:
        return self.navigator.get_leaf_context(leaf_id)

    def get_context(self, node_id: str) -> DirContext | FileContext | LeafContext:
        node = self.navigator.get_node(node_id)
        if node.node_type == "dir":
            return self.get_dir_context(node_id)
        if node.node_type == "file":
            return self.get_file_context(node_id)
        return self.get_leaf_context(node_id)

    def search_nodes(
        self,
        query: str = "",
        *,
        node_types: Iterable[NodeType] | None = None,
        leaf_kinds: Iterable[LeafKind] | None = None,
        location_prefix: str | None = None,
        limit: int | None = None,
    ) -> list[NodeContextRef]:
        return self.navigator.search_nodes(
            query=query,
            node_types=set(node_types) if node_types is not None else None,
            leaf_kinds=set(leaf_kinds) if leaf_kinds is not None else None,
            location_prefix=location_prefix,
            limit=limit,
        )

    def build_handoff_packet(
        self,
        *,
        task_id: str,
        task_intent: str,
        task_title: str = "",
        root_selectors: Sequence[str],
        target_selectors: Sequence[str],
        write_selectors: Sequence[str],
        read_only_selectors: Sequence[str] = (),
        locked_selectors: Sequence[str] = (),
        expansion_selectors: Sequence[str] = (),
        risks: Sequence[HandoffRisk | dict[str, Any]] = (),
        constraints: Sequence[HandoffConstraint | dict[str, Any]] = (),
        knowledge_dir: str | None = None,
        lineage_pack_selectors: Sequence[str] | None = None,
    ) -> WorkerHandoffPacket:
        root_nodes = self._resolve_handoff_refs(root_selectors, role="root")
        target_nodes = self._resolve_handoff_refs(target_selectors, role="target")
        write_nodes = self._resolve_handoff_refs(write_selectors, role="write")
        read_only_nodes = self._resolve_handoff_refs(
            read_only_selectors,
            role="read_only",
        )
        locked_nodes = self._resolve_handoff_refs(locked_selectors, role="locked")
        expansion_handles = self._resolve_handoff_refs(
            expansion_selectors,
            role="expansion",
        )
        return WorkerHandoffPacket(
            task_id=task_id,
            task_title=task_title,
            task_intent=task_intent,
            root_nodes=root_nodes,
            target_nodes=target_nodes,
            write_nodes=write_nodes,
            read_only_nodes=read_only_nodes,
            locked_nodes=locked_nodes,
            expansion_handles=expansion_handles,
            risks=[HandoffRisk.model_validate(item) for item in risks],
            constraints=[
                HandoffConstraint.model_validate(item) for item in constraints
            ],
            knowledge_dir=knowledge_dir or "",
            lineage_pack_selectors=list(
                lineage_pack_selectors
                if lineage_pack_selectors is not None
                else _merge_unique_selectors(
                    [ref.selector for ref in root_nodes],
                    [ref.selector for ref in target_nodes],
                    [ref.selector for ref in write_nodes],
                    [ref.selector for ref in read_only_nodes],
                    [ref.selector for ref in locked_nodes],
                    [ref.selector for ref in expansion_handles],
                )
            ),
        )

    def _summary_for_file(self, node: FileNode) -> FileSummaryV1 | None:
        if node.source_blob_hash is None:
            return None
        return self.file_summaries_by_hash.get(node.source_blob_hash)

    def _resolve_handoff_refs(
        self,
        selectors: Sequence[str],
        *,
        role: str,
    ) -> list[HandoffNodeRef]:
        refs: list[HandoffNodeRef] = []
        for selector in selectors:
            node = self.resolve_selector(selector)
            refs.append(
                HandoffNodeRef(
                    id=node.id,
                    selector=self.selector_for_node(node),
                    role=role,
                    name=node.name,
                    node_type=node.node_type,
                    location=node.location,
                    description=node.description,
                    kind=node.kind if node.node_type == "leaf" else None,
                )
            )
        return refs


def load_graph_context_service(knowledge_dir: Path | str) -> GraphContextService:
    return GraphContextService.from_knowledge_dir(knowledge_dir)


def _load_file_summaries(files_dir: Path) -> dict[str, FileSummaryV1]:
    summaries: dict[str, FileSummaryV1] = {}
    if not files_dir.exists():
        return summaries

    for path in sorted(files_dir.glob("*.json")):
        try:
            summary = FileSummaryV1.model_validate(
                json.loads(path.read_text(encoding="utf-8"))
            )
        except (OSError, ValueError) as exc:
            logger.warning("Could not load file summary artifact %s: %s", path, exc)
            continue
        summaries[summary.hash] = summary

    return summaries


def _symbol_to_ref(
    file_node: FileNode, summary_path: str, symbol: FileSymbolV1
) -> NodeContextRef:
    location = f"{summary_path}#{symbol.name}"
    return NodeContextRef(
        id=node_id("leaf", location, symbol.kind),
        name=symbol.name,
        node_type="leaf",
        location=location,
        language=file_node.language,
        description=symbol.description,
        parent_id=file_node.id,
        kind=symbol.kind,
    )


def _parse_selector(selector: str) -> tuple[str, str, str | None]:
    if selector.startswith("dir:"):
        return "dir", selector[4:], None
    if selector.startswith("file:"):
        return "file", selector[5:], None
    if selector.startswith("leaf:"):
        remainder = selector[5:]
        location, sep, kind = remainder.rpartition(":")
        if not sep or not location or not kind:
            raise ValueError(
                f"Invalid leaf selector, expected leaf:<path>#<symbol>:<kind>: {selector}"
            )
        return "leaf", location, kind
    raise ValueError(
        f"Unsupported selector format, expected dir:, file:, leaf:, or a node id: {selector}"
    )


def _merge_unique_selectors(*selector_groups: Sequence[str]) -> list[str]:
    ordered: list[str] = []
    seen: set[str] = set()
    for group in selector_groups:
        for selector in group:
            if selector in seen:
                continue
            seen.add(selector)
            ordered.append(selector)
    return ordered
