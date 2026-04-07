from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from orbit_agent.schemas.graph.nodes import (
    CodebaseGraphV1,
    DirNode,
    FileNode,
    LeafNode,
)

GRAPH_STORE_SCHEMA_VERSION = 1


class GraphObjectStore:
    def __init__(self, graph_dir: Path | str):
        self.graph_dir = Path(graph_dir)

    @property
    def refs_dir(self) -> Path:
        return self.graph_dir / "refs"

    @property
    def index_dir(self) -> Path:
        return self.graph_dir / "index"

    @property
    def objects_dir(self) -> Path:
        return self.graph_dir / "objects"

    @property
    def blobs_dir(self) -> Path:
        return self.graph_dir / "blobs"

    @property
    def current_ref_path(self) -> Path:
        return self.refs_dir / "current.json"

    @property
    def by_id_index_path(self) -> Path:
        return self.index_dir / "by-id.json"

    def write_graph(
        self,
        graph: CodebaseGraphV1,
        *,
        repo_path: Path | None = None,
    ) -> str:
        self._ensure_dirs()
        nodes = {
            **{node.id: node for node in graph.dirs},
            **{node.id: node for node in graph.files},
            **{node.id: node for node in graph.leaves},
        }
        object_hashes: dict[str, str] = {}
        index_nodes: dict[str, dict[str, Any]] = {}

        def write_node(node_id: str) -> str:
            if node_id in object_hashes:
                return object_hashes[node_id]

            node = nodes[node_id]
            child_ids = _child_ids(node)
            child_hashes = {child_id: write_node(child_id) for child_id in child_ids}
            node_data = node.model_dump(mode="json", exclude={"object_hash"})

            if isinstance(node, FileNode) and repo_path is not None:
                source_path = repo_path / node.location
                if source_path.exists():
                    source = source_path.read_text(encoding="utf-8", errors="replace")
                    node.source_blob_hash = self._write_blob(source)
                    node_data["source_blob_hash"] = node.source_blob_hash

            if isinstance(node, LeafNode) and node.source:
                node.source_blob_hash = self._write_blob(node.source)
                node_data["source_blob_hash"] = node.source_blob_hash
                node_data["source"] = ""
                for history_item in node_data.get("history", []):
                    source = history_item.get("source", "")
                    if source:
                        history_item["source_blob_hash"] = self._write_blob(source)
                        history_item["source"] = ""

            payload = {
                "schema_version": GRAPH_STORE_SCHEMA_VERSION,
                "object_type": "graph_node",
                "node_type": node.node_type,
                "node": node_data,
                "child_object_hashes": child_hashes,
            }
            object_hash = self._write_json_object(payload)
            node.object_hash = object_hash
            object_hashes[node_id] = object_hash
            index_nodes[node_id] = {
                "object_hash": object_hash,
                "node_type": node.node_type,
                "location": node.location,
                "identity_key": node.identity_key,
                "kind": node.kind if isinstance(node, LeafNode) else None,
            }
            return object_hash

        root_object_hash = write_node(graph.root_dir_id)
        root_payload = {
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "object_type": "codebase_graph",
            "root_dir_id": graph.root_dir_id,
            "root_object_hash": root_object_hash,
            "dirs": [node.id for node in graph.dirs],
            "files": [node.id for node in graph.files],
            "leaves": [node.id for node in graph.leaves],
            "node_count": len(nodes),
        }
        root_graph_hash = self._write_json_object(root_payload)

        by_id_index = {
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "root_dir_id": graph.root_dir_id,
            "root_graph_hash": root_graph_hash,
            "root_object_hash": root_object_hash,
            "dirs": [node.id for node in graph.dirs],
            "files": [node.id for node in graph.files],
            "leaves": [node.id for node in graph.leaves],
            "nodes": index_nodes,
        }
        self._write_json_file(self.by_id_index_path, by_id_index)

        current_ref = {
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "root_graph_hash": root_graph_hash,
            "root_object_hash": root_object_hash,
            "root_dir_id": graph.root_dir_id,
            "index": "graph/index/by-id.json",
        }
        self._write_json_file(self.current_ref_path, current_ref)
        return root_graph_hash

    def read_graph(self) -> CodebaseGraphV1:
        if not self.current_ref_path.exists():
            raise FileNotFoundError(
                f"Graph store entrypoint not found: {self.current_ref_path}"
            )
        if not self.by_id_index_path.exists():
            raise FileNotFoundError(
                f"Graph node index not found: {self.by_id_index_path}"
            )

        index = json.loads(self.by_id_index_path.read_text(encoding="utf-8"))
        nodes_by_id = index["nodes"]

        dirs = [
            DirNode.model_validate(self._read_node_data(nodes_by_id[node_id]))
            for node_id in index["dirs"]
        ]
        files = [
            FileNode.model_validate(self._read_node_data(nodes_by_id[node_id]))
            for node_id in index["files"]
        ]
        leaves = [
            LeafNode.model_validate(self._read_node_data(nodes_by_id[node_id]))
            for node_id in index["leaves"]
        ]
        return CodebaseGraphV1(
            root_dir_id=index["root_dir_id"],
            dirs=dirs,
            files=files,
            leaves=leaves,
        )

    def _read_node_data(self, index_entry: dict[str, Any]) -> dict[str, Any]:
        payload = self._read_json_object(index_entry["object_hash"])
        node_data = payload["node"]
        node_data["object_hash"] = index_entry["object_hash"]
        if payload["node_type"] == "leaf":
            source_blob_hash = node_data.get("source_blob_hash")
            if source_blob_hash:
                node_data["source"] = self._read_blob(source_blob_hash)
            for history_item in node_data.get("history", []):
                source_blob_hash = history_item.get("source_blob_hash")
                if source_blob_hash:
                    history_item["source"] = self._read_blob(source_blob_hash)
        return node_data

    def _ensure_dirs(self) -> None:
        for directory in [
            self.refs_dir,
            self.index_dir,
            self.objects_dir,
            self.blobs_dir,
        ]:
            directory.mkdir(parents=True, exist_ok=True)

    def _write_json_object(self, payload: dict[str, Any]) -> str:
        content = _canonical_json(payload)
        digest = _sha256(content.encode("utf-8"))
        path = self._object_path(digest)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
        return digest

    def _read_json_object(self, object_hash: str) -> dict[str, Any]:
        path = self._object_path(object_hash)
        if not path.exists():
            raise FileNotFoundError(f"Graph object not found: {path}")
        return json.loads(path.read_text(encoding="utf-8"))

    def _write_blob(self, content: str) -> str:
        digest = _sha256(content.encode("utf-8"))
        path = self._blob_path(digest)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return digest

    def _read_blob(self, blob_hash: str) -> str:
        path = self._blob_path(blob_hash)
        if not path.exists():
            raise FileNotFoundError(f"Graph blob not found: {path}")
        return path.read_text(encoding="utf-8")

    def _write_json_file(self, path: Path, payload: dict[str, Any]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")

    def _object_path(self, object_hash: str) -> Path:
        return self.objects_dir / object_hash[:2] / f"{object_hash}.json"

    def _blob_path(self, blob_hash: str) -> Path:
        return self.blobs_dir / blob_hash[:2] / f"{blob_hash}.txt"


def _child_ids(node: DirNode | FileNode | LeafNode) -> list[str]:
    if isinstance(node, DirNode):
        return [*node.dir_children, *node.file_children]
    if isinstance(node, FileNode):
        return list(node.leaf_children)
    return list(node.children)


def _canonical_json(payload: dict[str, Any]) -> str:
    return json.dumps(
        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )


def _sha256(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()
