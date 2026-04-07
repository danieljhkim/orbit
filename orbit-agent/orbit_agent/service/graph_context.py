from __future__ import annotations

from pathlib import Path
from typing import Iterable

from orbit_agent.graph.store import GraphObjectStore
from orbit_agent.schemas import (
    CodebaseGraphV1,
    DirContext,
    FileContext,
    LeafContext,
    NodeContextRef,
)
from orbit_agent.schemas.graph.contexts import NodeType
from orbit_agent.schemas.graph.navigation import GraphNavigator, GraphNode
from orbit_agent.schemas.graph.nodes import LeafKind


class GraphContextService:
    def __init__(self, graph: CodebaseGraphV1):
        self.graph = graph
        self.navigator = GraphNavigator(graph)

    @classmethod
    def from_graph_dir(cls, graph_dir: Path | str) -> GraphContextService:
        return cls(GraphObjectStore(graph_dir).read_graph())

    @classmethod
    def from_knowledge_dir(cls, knowledge_dir: Path | str) -> GraphContextService:
        return cls.from_graph_dir(Path(knowledge_dir) / "graph")

    def get_node(self, node_id: str) -> GraphNode:
        return self.navigator.get_node(node_id)

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
        return self.navigator.get_file_context(file_id)

    def get_leaf_context(self, leaf_id: str) -> LeafContext:
        return self.navigator.get_leaf_context(leaf_id)

    def get_context(self, node_id: str) -> DirContext | FileContext | LeafContext:
        return self.navigator.get_context(node_id)

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


def load_graph_context_service(knowledge_dir: Path | str) -> GraphContextService:
    return GraphContextService.from_knowledge_dir(knowledge_dir)
