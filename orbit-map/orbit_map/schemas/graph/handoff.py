from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field

from orbit_map.schemas.graph.contexts import NodeType
from orbit_map.schemas.graph.nodes import LeafKind

HandoffNodeRole = Literal[
    "root",
    "target",
    "write",
    "read_only",
    "locked",
    "expansion",
]
RiskSeverity = Literal["low", "medium", "high"]


class HandoffNodeRef(BaseModel):
    id: str
    selector: str
    role: HandoffNodeRole
    name: str
    node_type: NodeType
    location: str
    description: str = ""
    kind: LeafKind | None = None

    def to_markdown_line(self) -> str:
        parts = [f"`{self.selector}`", f"(id: `{self.id}`)", f"type: {self.node_type}"]
        if self.kind is not None:
            parts.append(f"kind: {self.kind}")
        line = f"- {' | '.join(parts)}"
        if self.description:
            line += f" | {self.description}"
        return line


class HandoffRisk(BaseModel):
    severity: RiskSeverity = "medium"
    description: str
    affected_selectors: list[str] = Field(default_factory=list)

    def to_markdown_line(self) -> str:
        selectors = ", ".join(f"`{selector}`" for selector in self.affected_selectors)
        scope = selectors or "(none)"
        return f"- [{self.severity}] {self.description} | Affected: {scope}"


class HandoffConstraint(BaseModel):
    description: str
    selectors: list[str] = Field(default_factory=list)

    def to_markdown_line(self) -> str:
        selectors = ", ".join(f"`{selector}`" for selector in self.selectors)
        scope = selectors or "(none)"
        return f"- {self.description} | Scope: {scope}"


class WorkerHandoffPacket(BaseModel):
    task_id: str
    task_title: str = ""
    task_intent: str
    root_nodes: list[HandoffNodeRef] = Field(default_factory=list)
    target_nodes: list[HandoffNodeRef] = Field(default_factory=list)
    write_nodes: list[HandoffNodeRef] = Field(default_factory=list)
    read_only_nodes: list[HandoffNodeRef] = Field(default_factory=list)
    locked_nodes: list[HandoffNodeRef] = Field(default_factory=list)
    expansion_handles: list[HandoffNodeRef] = Field(default_factory=list)
    risks: list[HandoffRisk] = Field(default_factory=list)
    constraints: list[HandoffConstraint] = Field(default_factory=list)
    knowledge_dir: str = ""
    lineage_pack_selectors: list[str] = Field(default_factory=list)

    def navigation_selectors(self) -> list[str]:
        if self.lineage_pack_selectors:
            return _dedupe_strings(self.lineage_pack_selectors)
        selectors = [
            ref.selector
            for ref in [
                *self.root_nodes,
                *self.target_nodes,
                *self.write_nodes,
                *self.read_only_nodes,
                *self.locked_nodes,
                *self.expansion_handles,
            ]
        ]
        return _dedupe_strings(selectors)

    def to_markdown(self, budget: int = 8_000) -> str:
        writer = _MarkdownWriter(budget=max(1, budget))

        writer.line("# Worker Handoff")
        writer.line()
        writer.line("## Task")
        writer.line(f"- task id: `{self.task_id}`")
        writer.line(f"- task title: {self.task_title or '(none)'}")
        writer.line(f"- task intent: {self.task_intent}")
        writer.line()
        writer.line("## Graph Scope")
        _write_node_section(writer, "Root Nodes", self.root_nodes)
        _write_node_section(writer, "Target Nodes", self.target_nodes)
        _write_node_section(writer, "Write Nodes", self.write_nodes)
        _write_node_section(writer, "Read-Only Nodes", self.read_only_nodes)
        _write_node_section(writer, "Locked Nodes", self.locked_nodes)
        _write_node_section(writer, "Expansion Handles", self.expansion_handles)
        if not writer.truncated:
            writer.line("## Risks")
            if self.risks:
                for risk in self.risks:
                    if not writer.line(risk.to_markdown_line()):
                        break
            else:
                writer.line("- (none)")
            writer.line()
        if not writer.truncated:
            writer.line("## Constraints")
            if self.constraints:
                for constraint in self.constraints:
                    if not writer.line(constraint.to_markdown_line()):
                        break
            else:
                writer.line("- (none)")
            writer.line()
        if not writer.truncated:
            writer.line("## Navigation")
            if self.knowledge_dir:
                writer.line(f"- knowledge dir: `{self.knowledge_dir}`")
            else:
                writer.line("- knowledge dir: (none)")
            selectors = self.navigation_selectors()
            writer.line(
                "- lineage pack selectors: "
                + (
                    ", ".join(f"`{selector}`" for selector in selectors)
                    if selectors
                    else "(none)"
                )
            )

        if writer.truncated:
            writer.note("_output truncated to respect the requested budget._")

        return writer.text()


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


def _dedupe_strings(values: list[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        ordered.append(value)
    return ordered


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
