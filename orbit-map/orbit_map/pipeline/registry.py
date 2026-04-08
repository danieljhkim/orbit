from __future__ import annotations

from collections.abc import Iterable

from orbit_map.pipeline.components import BUILTIN_COMPONENTS, BaseComponent
from orbit_map.pipeline.config import ComponentConfig, PipelineConfig


class ComponentRegistry:
    def __init__(self) -> None:
        self._components: dict[str, type[BaseComponent]] = {}

    def register(self, component_cls: type[BaseComponent]) -> None:
        self._components[component_cls.name] = component_cls

    def register_many(self, component_classes: Iterable[type[BaseComponent]]) -> None:
        for component_cls in component_classes:
            self.register(component_cls)

    def names(self) -> list[str]:
        return sorted(self._components)

    def create(self, spec: ComponentConfig) -> BaseComponent:
        try:
            component_cls = self._components[spec.name]
        except KeyError as exc:
            available = ", ".join(self.names())
            raise ValueError(
                f"Unknown component '{spec.name}'. Available components: {available}"
            ) from exc
        return component_cls(**spec.options)

    def create_many(self, config: PipelineConfig) -> list[BaseComponent]:
        return [self.create(spec) for spec in config.components]


def build_default_registry() -> ComponentRegistry:
    registry = ComponentRegistry()
    registry.register_many(BUILTIN_COMPONENTS)
    return registry
