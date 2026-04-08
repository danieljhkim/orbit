from __future__ import annotations

import ast
import logging

from orbit_map.graph.extraction.base import (
    GraphExtractionInput,
    GraphExtractionResult,
    leaf_id,
    leaf_identity_key,
    leaf_location,
    source_hash,
)
from orbit_map.schemas import LeafNode, SignatureField

logger = logging.getLogger(__name__)


class PythonGraphExtractor:
    language = "python"

    def extract(self, input_data: GraphExtractionInput) -> GraphExtractionResult:
        try:
            tree = ast.parse(input_data.source)
        except SyntaxError as exc:
            logger.warning(
                "Failed to parse Python file for graph extraction %s: %s",
                input_data.path,
                exc,
            )
            return GraphExtractionResult()

        leaves = self._extract_leaves(input_data, tree)
        top_level_leaf_ids = [
            leaf.id for leaf in leaves if leaf.parent_id == input_data.file_id
        ]
        return GraphExtractionResult(
            imports=self._extract_imports(input_data, tree),
            exports=[
                leaf.name for leaf in leaves if leaf.parent_id == input_data.file_id
            ],
            leaves=leaves,
            top_level_leaf_ids=top_level_leaf_ids,
        )

    def _extract_imports(
        self, input_data: GraphExtractionInput, tree: ast.Module
    ) -> list[str]:
        imports: list[str] = []
        for node in tree.body:
            if not isinstance(node, (ast.Import, ast.ImportFrom)):
                continue
            import_source = ast.get_source_segment(input_data.source, node)
            if import_source:
                imports.append(import_source)
            elif isinstance(node, ast.Import):
                imports.append(
                    "import " + ", ".join(alias.name for alias in node.names)
                )
            else:
                module = "." * node.level + (node.module or "")
                imports.append(
                    "from "
                    + module
                    + " import "
                    + ", ".join(alias.name for alias in node.names)
                )
        return imports

    def _extract_leaves(
        self, input_data: GraphExtractionInput, tree: ast.Module
    ) -> list[LeafNode]:
        leaves: list[LeafNode] = []

        def visit(
            body: list[ast.stmt],
            parent_id: str,
            prefix: str = "",
            inside_class: bool = False,
        ) -> list[str]:
            child_ids: list[str] = []

            for node in body:
                if isinstance(node, ast.ClassDef):
                    qualified_name = f"{prefix}.{node.name}" if prefix else node.name
                    node_source = ast.get_source_segment(input_data.source, node) or ""
                    leaf = self._class_leaf(
                        input_data, node, qualified_name, node_source, parent_id
                    )
                    leaf.children = visit(
                        node.body, leaf.id, qualified_name, inside_class=True
                    )
                    leaves.append(leaf)
                    child_ids.append(leaf.id)
                elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    qualified_name = f"{prefix}.{node.name}" if prefix else node.name
                    node_source = ast.get_source_segment(input_data.source, node) or ""
                    leaf = self._function_leaf(
                        input_data,
                        node,
                        qualified_name,
                        node_source,
                        parent_id,
                        inside_class=inside_class,
                    )
                    leaf.children = visit(
                        node.body, leaf.id, qualified_name, inside_class=False
                    )
                    leaves.append(leaf)
                    child_ids.append(leaf.id)

            return child_ids

        visit(tree.body, input_data.file_id)
        return leaves

    def _class_leaf(
        self,
        input_data: GraphExtractionInput,
        node: ast.ClassDef,
        qualified_name: str,
        node_source: str,
        parent_id: str,
    ) -> LeafNode:
        return LeafNode(
            id=leaf_id(input_data.path, qualified_name, "class"),
            identity_key=leaf_identity_key(input_data.path, qualified_name, "class"),
            name=node.name,
            location=leaf_location(input_data.path, qualified_name),
            language=self.language,
            description=ast.get_docstring(node) or "",
            parent_id=parent_id,
            kind="class",
            source=node_source,
            source_hash=source_hash(node_source),
            file_hash_at_capture=input_data.file_hash,
            start_line=getattr(node, "lineno", None),
            end_line=getattr(node, "end_lineno", None),
        )

    def _function_leaf(
        self,
        input_data: GraphExtractionInput,
        node: ast.FunctionDef | ast.AsyncFunctionDef,
        qualified_name: str,
        node_source: str,
        parent_id: str,
        inside_class: bool,
    ) -> LeafNode:
        return LeafNode(
            id=leaf_id(
                input_data.path,
                qualified_name,
                "method" if inside_class else "function",
            ),
            identity_key=leaf_identity_key(
                input_data.path,
                qualified_name,
                "method" if inside_class else "function",
            ),
            name=node.name,
            location=leaf_location(input_data.path, qualified_name),
            language=self.language,
            description=ast.get_docstring(node) or "",
            parent_id=parent_id,
            kind="method" if inside_class else "function",
            source=node_source,
            source_hash=source_hash(node_source),
            file_hash_at_capture=input_data.file_hash,
            input_signature=self._function_inputs(node, input_data.source),
            output_signature=self._function_outputs(node, input_data.source),
            start_line=getattr(node, "lineno", None),
            end_line=getattr(node, "end_lineno", None),
        )

    def _function_inputs(
        self, node: ast.FunctionDef | ast.AsyncFunctionDef, source: str
    ) -> list[SignatureField]:
        items: list[SignatureField] = []

        def add_arg(arg: ast.arg, prefix: str = "") -> None:
            items.append(
                SignatureField(
                    name=f"{prefix}{arg.arg}",
                    annotation=self._annotation_to_str(arg.annotation, source),
                )
            )

        for arg in node.args.posonlyargs:
            add_arg(arg)
        for arg in node.args.args:
            add_arg(arg)
        if node.args.vararg is not None:
            add_arg(node.args.vararg, prefix="*")
        for arg in node.args.kwonlyargs:
            add_arg(arg)
        if node.args.kwarg is not None:
            add_arg(node.args.kwarg, prefix="**")
        return items

    def _function_outputs(
        self, node: ast.FunctionDef | ast.AsyncFunctionDef, source: str
    ) -> list[SignatureField]:
        annotation = self._annotation_to_str(node.returns, source)
        if annotation is None:
            return []
        return [SignatureField(name="return", annotation=annotation)]

    def _annotation_to_str(self, node: ast.AST | None, source: str) -> str | None:
        if node is None:
            return None
        segment = ast.get_source_segment(source, node)
        if segment:
            return segment
        try:
            return ast.unparse(node)
        except Exception:
            return None
