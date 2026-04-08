from __future__ import annotations

from orbit_map.schemas.graph.nodes import BaseNode, CodebaseGraphV1


def iter_nodes(graph: CodebaseGraphV1) -> list[BaseNode]:
    return [*graph.dirs, *graph.files, *graph.leaves]


def build_node_index(graph: CodebaseGraphV1) -> dict[str, BaseNode]:
    return {node.id: node for node in iter_nodes(graph)}


def lock_lineage(
    graph: CodebaseGraphV1,
    node_id: str,
    owner: str,
    reason: str = "",
) -> CodebaseGraphV1:
    index = build_node_index(graph)
    try:
        node = index[node_id]
    except KeyError as exc:
        raise ValueError(f"Unknown node id: {node_id}") from exc

    node.is_locked = True
    node.lock_owner = owner
    node.lock_reason = reason
    recompute_lineage_locks(graph)
    return graph


def unlock_lineage(
    graph: CodebaseGraphV1,
    node_id: str,
    owner: str | None = None,
) -> CodebaseGraphV1:
    index = build_node_index(graph)
    try:
        node = index[node_id]
    except KeyError as exc:
        raise ValueError(f"Unknown node id: {node_id}") from exc

    if owner is not None and node.lock_owner not in {None, owner}:
        raise ValueError(f"Node {node_id} is locked by {node.lock_owner}, not {owner}")

    node.is_locked = False
    node.lock_owner = None
    node.lock_reason = ""
    recompute_lineage_locks(graph)
    return graph


def recompute_lineage_locks(graph: CodebaseGraphV1) -> CodebaseGraphV1:
    index = build_node_index(graph)

    for node in index.values():
        node.lineage_locked = False

    for node in index.values():
        if not node.is_locked:
            continue

        current_parent_id = node.parent_id
        visited: set[str] = set()
        while current_parent_id is not None:
            if current_parent_id in visited:
                raise ValueError(
                    f"Cycle detected in graph lineage at node id: {current_parent_id}"
                )
            visited.add(current_parent_id)

            try:
                parent = index[current_parent_id]
            except KeyError as exc:
                raise ValueError(
                    f"Missing parent node id: {current_parent_id}"
                ) from exc

            if not parent.is_locked:
                parent.lineage_locked = True
            current_parent_id = parent.parent_id

    return graph
