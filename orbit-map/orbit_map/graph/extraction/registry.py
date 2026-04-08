from __future__ import annotations

from orbit_map.graph.extraction.base import GraphExtractor
from orbit_map.graph.extraction.python import PythonGraphExtractor


class GraphExtractorRegistry:
    def __init__(self) -> None:
        self._extractors: dict[str, GraphExtractor] = {}

    def register(self, extractor: GraphExtractor) -> None:
        self._extractors[extractor.language] = extractor

    def get(self, language: str) -> GraphExtractor | None:
        return self._extractors.get(language)

    def languages(self) -> list[str]:
        return sorted(self._extractors)


def build_default_extractor_registry() -> GraphExtractorRegistry:
    registry = GraphExtractorRegistry()
    registry.register(PythonGraphExtractor())
    return registry
