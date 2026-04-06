from __future__ import annotations

import logging
import json
from pathlib import Path

from orbit_agent.agent import BaseAgent, get_agent
from orbit_agent.pipeline.context import PipelineContext
from orbit_agent.schemas import (
    ArchitectureV1,
    FileSummaryV1,
    GenerateArchitectureInputV1,
    GenerateArchitectureResponseV1,
)

from .base import BaseComponent

logger = logging.getLogger(__name__)

ARCHITECTURE_SYSTEM_PROMPT = """\
You are a software architect. Given a list of file summaries from a codebase, produce a JSON object with:
- summary: 2-3 sentence overview of the system
- components: array of {name, role, depends_on: [string]} identifying major components/modules
- key_flows: array of {name, description, steps: [string]} describing important workflows

Be concise. Identify real architectural boundaries. Do not over-decompose.
Respond with ONLY valid JSON, no markdown fencing."""

FALLBACK = ArchitectureV1(summary="Failed to generate")


def _format_summaries(file_summaries: list[FileSummaryV1]) -> str:
    lines: list[str] = []
    for fs in file_summaries:
        symbol_names = ", ".join(symbol.name for symbol in fs.symbols)
        suffix = f" [symbols: {symbol_names}]" if symbol_names else ""
        lines.append(f"{fs.path}: {fs.summary}{suffix}")
    return "\n".join(lines)


class GenerateArchitectureComponent(BaseComponent):
    name = "generate_architecture"

    def __init__(self, agent: BaseAgent | None = None) -> None:
        self.agent = agent

    def _read(self, file_summaries: list[FileSummaryV1]) -> GenerateArchitectureInputV1:
        return GenerateArchitectureInputV1(file_summaries=file_summaries)

    def _transform(
        self, data: GenerateArchitectureInputV1
    ) -> GenerateArchitectureResponseV1:
        agent = self.agent or get_agent()
        user_message = _format_summaries(data.file_summaries)

        logger.info(
            "Generating architecture summary for %d files", len(data.file_summaries)
        )

        try:
            raw = agent.chat(ARCHITECTURE_SYSTEM_PROMPT, user_message)
            architecture = ArchitectureV1.model_validate_json(raw)
        except Exception:
            logger.warning("LLM response parse failed for architecture, using fallback")
            architecture = FALLBACK.model_copy(deep=True)

        return GenerateArchitectureResponseV1(architecture=architecture)

    def _write(
        self, response: GenerateArchitectureResponseV1, output_dir: Path
    ) -> None:
        output_dir.mkdir(parents=True, exist_ok=True)
        arch_path = output_dir / "architecture.json"
        arch_path.write_text(
            json.dumps(response.architecture.model_dump(mode="json"), indent=2) + "\n"
        )

    def execute(self, context: PipelineContext) -> PipelineContext:
        if context.summarize_response is None:
            raise ValueError(
                "GenerateArchitectureComponent requires summarize_response in the pipeline context"
            )
        if self.agent is None:
            self.agent = context.agent or get_agent()
        file_summaries = (
            context.summarize_response.files if context.summarize_response else []
        )
        data = self._read(file_summaries)
        response = self._transform(data)
        self._write(response, context.output_dir)
        context.architecture_response = response
        return context
