from __future__ import annotations

import asyncio
import hashlib
import json
import logging
from datetime import datetime, timezone
from pathlib import Path

from pydantic import ValidationError

from orbit_map.graph.store import GraphObjectStore
from orbit_map.pipeline.context import PipelineContext
from orbit_map.runtime.agent import BaseAgent, get_agent
from orbit_map.schemas import (
    FileSummaryAnalysisV1,
    FileSymbolV1,
    FileSummaryV1,
    SourceFileV1,
    SummarizeFilesInputV1,
    SummarizeFilesResponseV1,
)

from .base import BaseComponent

logger = logging.getLogger(__name__)

EXTENSION_MAP: dict[str, str] = {
    ".py": "python",
    ".rs": "rust",
    ".ts": "typescript",
    ".js": "javascript",
    ".go": "go",
    ".java": "java",
    ".md": "markdown",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".json": "json",
    ".toml": "toml",
}

SUMMARIZE_SYSTEM_PROMPT = """\
You are a code analyzer. Given a source file, produce a JSON object with these fields:
- summary: one-sentence description of what this file does
- symbols: array of {name, kind, signature, description} where kind is one of: function, struct, class, interface, module, method, field, trait, impl
- imports: array of import/dependency strings
- exports: array of exported symbol names

Be precise. Use exact identifiers from the source. Do not hallucinate names.
Respond with ONLY valid JSON, no markdown fencing."""


def _detect_language(file_path: str) -> str:
    ext = Path(file_path).suffix.lower()
    return EXTENSION_MAP.get(ext, ext.lstrip(".") if ext else "unknown")


SYMBOL_KIND_ALIASES = {
    "attribute": "field",
    "constant": "field",
    "dataclass": "class",
    "enum": "class",
    "file": "module",
    "object": "class",
    "package": "module",
    "property": "field",
    "type_alias": "module",
    "variable": "field",
}

VALID_SYMBOL_KINDS = {
    "function",
    "struct",
    "class",
    "interface",
    "module",
    "method",
    "field",
    "trait",
    "impl",
}


def _normalize_symbol_kind(value: object) -> str | None:
    if not isinstance(value, str):
        return None

    normalized = value.strip().lower().replace("-", "_").replace(" ", "_")
    normalized = SYMBOL_KIND_ALIASES.get(normalized, normalized)
    if normalized in VALID_SYMBOL_KINDS:
        return normalized
    return None


def _coerce_string_list(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item.strip() for item in value if isinstance(item, str) and item.strip()]


def _parse_analysis(raw: str, file_path: str) -> FileSummaryAnalysisV1:
    try:
        return FileSummaryAnalysisV1.model_validate_json(raw)
    except ValidationError as exc:
        logger.info("Strict summary validation failed for %s: %s", file_path, exc)
    except json.JSONDecodeError as exc:
        logger.warning("Summary JSON decode failed for %s: %s", file_path, exc)
        return FileSummaryAnalysisV1()

    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        logger.warning("Summary JSON decode failed for %s: %s", file_path, exc)
        return FileSummaryAnalysisV1()

    if not isinstance(payload, dict):
        logger.warning(
            "Summary payload for %s was not an object, using fallback", file_path
        )
        return FileSummaryAnalysisV1()

    symbols: list[FileSymbolV1] = []
    for item in payload.get("symbols", []):
        if not isinstance(item, dict):
            continue

        kind = _normalize_symbol_kind(item.get("kind"))
        name = item.get("name")
        if kind is None or not isinstance(name, str) or not name.strip():
            continue

        symbols.append(
            FileSymbolV1(
                name=name.strip(),
                kind=kind,
                signature=item.get("signature", "")
                if isinstance(item.get("signature"), str)
                else "",
                description=item.get("description", "")
                if isinstance(item.get("description"), str)
                else "",
            )
        )

    summary = payload.get("summary")
    return FileSummaryAnalysisV1(
        summary=summary
        if isinstance(summary, str) and summary.strip()
        else "Failed to summarize",
        symbols=symbols,
        imports=_coerce_string_list(payload.get("imports")),
        exports=_coerce_string_list(payload.get("exports")),
    )


class SummarizeFilesComponent(BaseComponent):
    name = "summarize_files"

    def __init__(
        self,
        agent: BaseAgent | None = None,
        delay: float = 0.0,
        max_concurrency: int = 8,
    ) -> None:
        self.agent = agent
        self.delay = delay
        self.max_concurrency = max(1, max_concurrency)

    def _read(self, file_paths: list[str], repo_path: Path) -> SummarizeFilesInputV1:
        total = len(file_paths)
        files: list[SourceFileV1] = []

        for i, fp in enumerate(file_paths, 1):
            abs_path = repo_path / fp
            relative_path = str(Path(fp))

            logger.info(
                "Reading file %d of %d for summarization: %s", i, total, relative_path
            )

            try:
                content_bytes = abs_path.read_bytes()
            except OSError:
                logger.warning("Could not read file: %s", abs_path)
                continue

            files.append(
                SourceFileV1(
                    path=relative_path,
                    hash=hashlib.sha256(content_bytes).hexdigest(),
                    language=_detect_language(fp),
                    content=content_bytes.decode("utf-8", errors="replace"),
                    metadata={
                        "size_bytes": len(content_bytes),
                        "last_modified": datetime.fromtimestamp(
                            abs_path.stat().st_mtime, tz=timezone.utc
                        ),
                    },
                )
            )

        return SummarizeFilesInputV1(files=files)

    def _read_missing_from_graph(
        self, context: PipelineContext
    ) -> SummarizeFilesInputV1:
        store = GraphObjectStore(context.graph_dir)
        if context.codebase_graph is None:
            context.codebase_graph = store.read_graph()

        files: list[SourceFileV1] = []
        for file_node in context.codebase_graph.files:
            if file_node.source_blob_hash is None:
                logger.warning(
                    "Skipping graph file without source blob hash: %s",
                    file_node.location,
                )
                continue

            summary_path = context.files_dir / f"{file_node.source_blob_hash}.json"
            if summary_path.exists():
                logger.debug(
                    "Skipping existing summary for graph file: %s",
                    file_node.location,
                )
                continue

            source = store.read_blob(file_node.source_blob_hash)
            abs_path = context.repo_path / file_node.location
            metadata = {
                "size_bytes": len(source.encode("utf-8")),
                "last_modified": datetime.fromtimestamp(0, tz=timezone.utc),
            }
            if abs_path.exists():
                metadata["last_modified"] = datetime.fromtimestamp(
                    abs_path.stat().st_mtime, tz=timezone.utc
                )

            files.append(
                SourceFileV1(
                    path=file_node.location,
                    hash=file_node.source_blob_hash,
                    language=file_node.language,
                    content=source,
                    metadata=metadata,
                )
            )

        logger.info("Selected %d graph file(s) for summarization", len(files))
        return SummarizeFilesInputV1(files=files)

    def _transform(self, data: SummarizeFilesInputV1) -> SummarizeFilesResponseV1:
        agent = self.agent or get_agent()
        return asyncio.run(self._transform_async(data, agent))

    async def _transform_async(
        self, data: SummarizeFilesInputV1, agent: BaseAgent
    ) -> SummarizeFilesResponseV1:
        total = len(data.files)
        semaphore = asyncio.Semaphore(self.max_concurrency)
        logger.info(
            "Summarizing %d file(s) with max concurrency %d",
            total,
            self.max_concurrency,
        )

        tasks = [
            self._summarize_one(agent, source_file, i, total, semaphore)
            for i, source_file in enumerate(data.files, 1)
        ]
        results = await asyncio.gather(*tasks)
        return SummarizeFilesResponseV1(files=list(results))

    async def _summarize_one(
        self,
        agent: BaseAgent,
        source_file: SourceFileV1,
        index: int,
        total: int,
        semaphore: asyncio.Semaphore,
    ) -> FileSummaryV1:
        if self.delay > 0 and index > 1:
            await asyncio.sleep(self.delay * (index - 1))

        async with semaphore:
            logger.info(
                "Summarizing file %d of %d: %s", index, total, source_file.path
            )
            user_message = f"File: {source_file.path}\n\n{source_file.content}"

            try:
                raw = await asyncio.to_thread(
                    agent.chat, SUMMARIZE_SYSTEM_PROMPT, user_message
                )
                analysis = _parse_analysis(raw, source_file.path)
            except Exception:
                logger.warning(
                    "LLM response parse failed for %s, using fallback", source_file.path
                )
                analysis = FileSummaryAnalysisV1()

            return FileSummaryV1(
                path=source_file.path,
                hash=source_file.hash,
                language=source_file.language,
                summary=analysis.summary,
                symbols=analysis.symbols,
                imports=analysis.imports,
                exports=analysis.exports,
                metadata=source_file.metadata,
            )

    def _write(self, response: SummarizeFilesResponseV1, output_dir: Path) -> None:
        output_dir.mkdir(parents=True, exist_ok=True)

        for summary in response.files:
            summary_path = output_dir / f"{summary.hash}.json"
            summary_path.write_text(
                json.dumps(summary.model_dump(mode="json"), indent=2) + "\n"
            )

    def execute(self, context: PipelineContext) -> PipelineContext:
        data = (
            self._read(context.changed_paths, context.repo_path)
            if context.changed_paths
            else self._read_missing_from_graph(context)
        )
        if not data.files:
            response = SummarizeFilesResponseV1()
        else:
            if self.agent is None:
                self.agent = context.agent or get_agent()
            response = self._transform(data)
        self._write(response, context.files_dir)
        context.summarize_response = response
        return context
