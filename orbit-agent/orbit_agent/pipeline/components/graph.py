from __future__ import annotations

import logging
from pathlib import Path

from orbit_agent.graph import detect_language
from orbit_agent.graph.extraction import (
    GraphExtractionInput,
    GraphExtractorRegistry,
    build_default_extractor_registry,
)
from orbit_agent.graph.extraction.base import identity_key, node_id
from orbit_agent.graph.store import GraphObjectStore
from orbit_agent.pipeline.context import PipelineContext
from orbit_agent.schemas import (
    CodebaseGraphV1,
    DirNode,
    FileNode,
    LeafNode,
)

from .base import BaseComponent

logger = logging.getLogger(__name__)


def _dir_id(path: str) -> str:
    return node_id("dir", path, "directory")


def _file_id(path: str) -> str:
    return node_id("file", path, "source")


class BuildGraphDirsComponent(BaseComponent):
    name = "build_graph_dirs"

    def execute(self, context: PipelineContext) -> PipelineContext:
        logger.info("Building graph directory nodes")
        root_location = "."
        root_id = _dir_id(root_location)

        dir_paths: set[Path] = {Path(".")}
        for file_path in context.file_paths:
            current = file_path.parent
            while True:
                dir_paths.add(current)
                if current == Path("."):
                    break
                current = current.parent

        sorted_dirs = sorted(dir_paths, key=lambda path: (len(path.parts), str(path)))
        dirs: dict[str, DirNode] = {}
        for path in sorted_dirs:
            location = "." if str(path) == "." else str(path)
            parent_id = None if location == "." else _dir_id(str(path.parent))
            dirs[location] = DirNode(
                id=_dir_id(location),
                identity_key=identity_key("dir", location, "directory"),
                name=context.repo_path.name if location == "." else path.name,
                location=location,
                language="mixed",
                parent_id=parent_id,
            )

        for location, node in dirs.items():
            if location == ".":
                continue
            parent_location = str(Path(location).parent)
            if parent_location == "":
                parent_location = "."
            dirs[parent_location].dir_children.append(node.id)

        context.codebase_graph = CodebaseGraphV1(
            root_dir_id=root_id, dirs=list(dirs.values())
        )
        logger.info(
            "Built %d graph directory node(s)", len(context.codebase_graph.dirs)
        )
        return context


class BuildGraphFilesComponent(BaseComponent):
    name = "build_graph_files"

    def execute(self, context: PipelineContext) -> PipelineContext:
        if context.codebase_graph is None:
            raise ValueError(
                "BuildGraphFilesComponent requires codebase_graph in the pipeline context"
            )

        logger.info("Building graph file nodes")
        dir_index = {node.location: node for node in context.codebase_graph.dirs}
        files: list[FileNode] = []

        for file_path in context.file_paths:
            location = str(file_path)
            parent_location = str(file_path.parent)
            if parent_location == "":
                parent_location = "."
            file_node = FileNode(
                id=_file_id(location),
                identity_key=identity_key("file", location, "source"),
                name=file_path.name,
                location=location,
                language=detect_language(location),
                parent_id=_dir_id(parent_location),
                extension=file_path.suffix.lower() or None,
            )
            files.append(file_node)
            dir_index[parent_location].file_children.append(file_node.id)

        context.codebase_graph.files = files
        logger.info("Built %d graph file node(s)", len(files))
        return context


class BuildGraphLeavesComponent(BaseComponent):
    name = "build_graph_leaves"

    def __init__(
        self, extractor_registry: GraphExtractorRegistry | None = None
    ) -> None:
        self.extractor_registry = (
            extractor_registry or build_default_extractor_registry()
        )

    def execute(self, context: PipelineContext) -> PipelineContext:
        if context.codebase_graph is None:
            raise ValueError(
                "BuildGraphLeavesComponent requires codebase_graph in the pipeline context"
            )

        logger.info(
            "Building graph leaf nodes with extractor(s): %s",
            ", ".join(self.extractor_registry.languages()) or "none",
        )
        file_index = {node.location: node for node in context.codebase_graph.files}
        leaves: list[LeafNode] = []

        for file_path in context.file_paths:
            location = str(file_path)
            file_node = file_index[location]
            extractor = self.extractor_registry.get(file_node.language)
            if extractor is None:
                logger.debug(
                    "Skipping leaf extraction for %s: no extractor registered for %s",
                    location,
                    file_node.language,
                )
                continue

            abs_path = context.repo_path / file_path
            try:
                source = abs_path.read_text(encoding="utf-8", errors="replace")
            except OSError as exc:
                logger.warning(
                    "Could not read %s for graph leaf extraction: %s", abs_path, exc
                )
                continue

            result = extractor.extract(
                GraphExtractionInput(
                    path=location,
                    source=source,
                    file_id=file_node.id,
                    file_hash=context.new_hashes.get(location),
                )
            )
            file_node.imports = result.imports
            file_node.exports = result.exports
            file_node.leaf_children.extend(result.top_level_leaf_ids)
            leaves.extend(result.leaves)

        context.codebase_graph.leaves = leaves
        logger.info("Built %d graph leaf node(s)", len(leaves))
        return context


class PersistGraphComponent(BaseComponent):
    name = "persist_graph"

    def execute(self, context: PipelineContext) -> PipelineContext:
        if context.codebase_graph is None:
            raise ValueError(
                "PersistGraphComponent requires codebase_graph in the pipeline context"
            )

        logger.info("Persisting codebase graph to %s", context.graph_dir)
        legacy_graph_path = context.output_dir / "graph.json"
        if legacy_graph_path.exists():
            legacy_graph_path.unlink()
        store = GraphObjectStore(context.graph_dir)
        store.write_graph(
            context.codebase_graph,
            repo_path=context.repo_path,
        )
        return context
