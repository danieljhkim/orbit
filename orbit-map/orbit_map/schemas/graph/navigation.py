from __future__ import annotations

from orbit_map.schemas.graph.contexts import (
    DirContext,
    FileContext,
    LeafContext,
    NodeContextRef,
    NodeLockState,
    NodeType,
)
from orbit_map.schemas.graph.locking import build_node_index
from orbit_map.schemas.graph.nodes import (
    BaseNode,
    CodebaseGraphV1,
    DirNode,
    FileNode,
    LeafKind,
    LeafNode,
)

GraphNode = DirNode | FileNode | LeafNode


class GraphNavigator:
    def __init__(self, graph: CodebaseGraphV1):
        self.graph = graph
        self.node_index = build_node_index(graph)

    def get_root(self) -> DirNode:
        return self.get_dir(self.graph.root_dir_id)

    def get_node(self, node_id: str) -> GraphNode:
        node = self.node_index.get(node_id)
        if node is None:
            raise ValueError(f"Unknown node id: {node_id}")
        return self._as_graph_node(node)

    def get_dir(self, node_id: str) -> DirNode:
        node = self.get_node(node_id)
        if not isinstance(node, DirNode):
            raise ValueError(f"Expected dir node, got {node.node_type}: {node_id}")
        return node

    def get_file(self, node_id: str) -> FileNode:
        node = self.get_node(node_id)
        if not isinstance(node, FileNode):
            raise ValueError(f"Expected file node, got {node.node_type}: {node_id}")
        return node

    def get_leaf(self, node_id: str) -> LeafNode:
        node = self.get_node(node_id)
        if not isinstance(node, LeafNode):
            raise ValueError(f"Expected leaf node, got {node.node_type}: {node_id}")
        return node

    def get_parent(self, node_id: str) -> GraphNode | None:
        node = self.get_node(node_id)
        if node.parent_id is None:
            return None
        return self.get_node(node.parent_id)

    def get_children(self, node_id: str) -> list[GraphNode]:
        node = self.get_node(node_id)
        child_ids: list[str]
        if isinstance(node, DirNode):
            child_ids = [*node.dir_children, *node.file_children]
        elif isinstance(node, FileNode):
            child_ids = list(node.leaf_children)
        else:
            child_ids = list(node.children)
        return [self.get_node(child_id) for child_id in child_ids]

    def get_siblings(self, node_id: str) -> list[GraphNode]:
        node = self.get_node(node_id)
        parent = self.get_parent(node_id)
        if parent is None:
            return []
        return [
            sibling for sibling in self.get_children(parent.id) if sibling.id != node.id
        ]

    def get_lineage(self, node_id: str, include_self: bool = False) -> list[GraphNode]:
        current = self.get_node(node_id) if include_self else self.get_parent(node_id)
        lineage: list[GraphNode] = []
        visited: set[str] = set()

        while current is not None:
            if current.id in visited:
                raise ValueError(
                    f"Cycle detected in graph lineage at node id: {current.id}"
                )
            visited.add(current.id)
            lineage.append(current)
            current = self.get_parent(current.id)

        lineage.reverse()
        return lineage

    def get_containing_file(self, node_id: str) -> FileNode | None:
        current = self.get_node(node_id)
        visited: set[str] = set()

        while current.parent_id is not None:
            if current.id in visited:
                raise ValueError(
                    f"Cycle detected in graph lineage at node id: {current.id}"
                )
            visited.add(current.id)
            parent = self.get_parent(current.id)
            if parent is None:
                return None
            if isinstance(parent, FileNode):
                return parent
            current = parent

        return None

    def search_nodes(
        self,
        query: str = "",
        *,
        node_types: set[NodeType] | None = None,
        leaf_kinds: set[LeafKind] | None = None,
        location_prefix: str | None = None,
        limit: int | None = None,
    ) -> list[NodeContextRef]:
        normalized_query = query.strip().lower()
        prefix = location_prefix.strip("/") if location_prefix else None
        matches: list[tuple[tuple[int, int, str], GraphNode]] = []

        for node in self.node_index.values():
            graph_node = self._as_graph_node(node)
            if node_types is not None and graph_node.node_type not in node_types:
                continue
            if leaf_kinds is not None:
                if (
                    not isinstance(graph_node, LeafNode)
                    or graph_node.kind not in leaf_kinds
                ):
                    continue
            if prefix is not None and not graph_node.location.startswith(prefix):
                continue

            haystack = " ".join(
                part
                for part in [
                    graph_node.id,
                    graph_node.name,
                    graph_node.location,
                    graph_node.description,
                    graph_node.kind if isinstance(graph_node, LeafNode) else "",
                ]
                if part
            ).lower()
            if normalized_query and normalized_query not in haystack:
                continue

            score = self._search_score(graph_node, normalized_query)
            matches.append((score, graph_node))

        matches.sort(key=lambda item: item[0])
        refs = [self.to_ref(node) for _, node in matches]
        if limit is not None:
            return refs[:limit]
        return refs

    def get_dir_context(self, dir_id: str) -> DirContext:
        node = self.get_dir(dir_id)
        parent = self.get_parent(node.id)
        if parent is not None and not isinstance(parent, DirNode):
            raise ValueError(
                f"Dir node {node.id} must have a dir parent, got {parent.node_type}"
            )

        return DirContext(
            node=node,
            lock=self.to_lock(node),
            parent=self.to_ref(parent) if parent is not None else None,
            lineage=self.to_refs(self.get_lineage(node.id)),
            child_dirs=self.to_refs(self._nodes_from_ids(node.dir_children)),
            child_files=self.to_refs(self._nodes_from_ids(node.file_children)),
            summary=node.description,
        )

    def get_file_context(self, file_id: str) -> FileContext:
        node = self.get_file(file_id)
        parent = self.get_parent(node.id)
        if parent is not None and not isinstance(parent, DirNode):
            raise ValueError(
                f"File node {node.id} must have a dir parent, got {parent.node_type}"
            )

        return FileContext(
            node=node,
            lock=self.to_lock(node),
            parent_dir=self.to_ref(parent) if parent is not None else None,
            lineage=self.to_refs(self.get_lineage(node.id)),
            imports=list(node.imports),
            exports=list(node.exports),
            top_level_leaves=self.to_refs(self._nodes_from_ids(node.leaf_children)),
            summary=node.description,
        )

    def get_leaf_context(self, leaf_id: str) -> LeafContext:
        node = self.get_leaf(leaf_id)
        containing_file = self.get_containing_file(node.id)

        return LeafContext(
            node=node,
            lock=self.to_lock(node),
            parent_file=self.to_ref(containing_file)
            if containing_file is not None
            else None,
            lineage=self.to_refs(self.get_lineage(node.id)),
            child_leaves=self.to_refs(self._nodes_from_ids(node.children)),
            siblings=self.to_refs(self.get_siblings(node.id)),
            history=list(node.history),
        )

    def get_context(self, node_id: str) -> DirContext | FileContext | LeafContext:
        node = self.get_node(node_id)
        if isinstance(node, DirNode):
            return self.get_dir_context(node_id)
        if isinstance(node, FileNode):
            return self.get_file_context(node_id)
        return self.get_leaf_context(node_id)

    def _nodes_from_ids(self, node_ids: list[str]) -> list[GraphNode]:
        return [self.get_node(node_id) for node_id in node_ids]

    def to_ref(self, node: GraphNode) -> NodeContextRef:
        return NodeContextRef(
            id=node.id,
            name=node.name,
            node_type=self._node_type(node),
            location=node.location,
            language=node.language,
            description=node.description,
            parent_id=node.parent_id,
            kind=node.kind if isinstance(node, LeafNode) else None,
        )

    def to_refs(self, nodes: list[GraphNode]) -> list[NodeContextRef]:
        return [self.to_ref(node) for node in nodes]

    def to_lock(self, node: GraphNode) -> NodeLockState:
        return NodeLockState(
            is_locked=node.is_locked,
            lineage_locked=node.lineage_locked,
            lock_owner=node.lock_owner,
            lock_reason=node.lock_reason,
        )

    def _search_score(
        self, node: GraphNode, normalized_query: str
    ) -> tuple[int, int, str]:
        if not normalized_query:
            return (2, len(node.location), node.location)

        name = node.name.lower()
        location = node.location.lower()
        if name == normalized_query:
            return (0, len(node.location), node.location)
        if name.startswith(normalized_query):
            return (1, len(node.location), node.location)
        if location.startswith(normalized_query):
            return (2, len(node.location), node.location)
        return (3, len(node.location), node.location)

    def _node_type(self, node: GraphNode) -> NodeType:
        return node.node_type

    def _as_graph_node(self, node: BaseNode) -> GraphNode:
        if not isinstance(node, (DirNode, FileNode, LeafNode)):
            raise ValueError(f"Unsupported graph node type: {type(node).__name__}")
        return node


def get_node(graph: CodebaseGraphV1, node_id: str) -> GraphNode:
    return GraphNavigator(graph).get_node(node_id)


def get_parent(graph: CodebaseGraphV1, node_id: str) -> GraphNode | None:
    return GraphNavigator(graph).get_parent(node_id)


def get_children(graph: CodebaseGraphV1, node_id: str) -> list[GraphNode]:
    return GraphNavigator(graph).get_children(node_id)


def get_siblings(graph: CodebaseGraphV1, node_id: str) -> list[GraphNode]:
    return GraphNavigator(graph).get_siblings(node_id)


def get_lineage(
    graph: CodebaseGraphV1, node_id: str, include_self: bool = False
) -> list[GraphNode]:
    return GraphNavigator(graph).get_lineage(node_id, include_self=include_self)


def search_nodes(
    graph: CodebaseGraphV1,
    query: str = "",
    *,
    node_types: set[NodeType] | None = None,
    leaf_kinds: set[LeafKind] | None = None,
    location_prefix: str | None = None,
    limit: int | None = None,
) -> list[NodeContextRef]:
    return GraphNavigator(graph).search_nodes(
        query=query,
        node_types=node_types,
        leaf_kinds=leaf_kinds,
        location_prefix=location_prefix,
        limit=limit,
    )


def build_dir_context(graph: CodebaseGraphV1, dir_id: str) -> DirContext:
    return GraphNavigator(graph).get_dir_context(dir_id)


def build_file_context(graph: CodebaseGraphV1, file_id: str) -> FileContext:
    return GraphNavigator(graph).get_file_context(file_id)


def build_leaf_context(graph: CodebaseGraphV1, leaf_id: str) -> LeafContext:
    return GraphNavigator(graph).get_leaf_context(leaf_id)
