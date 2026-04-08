from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


class ComponentConfig(BaseModel):
    name: str
    options: dict[str, Any] = Field(default_factory=dict)


class PipelineConfig(BaseModel):
    components: list[ComponentConfig] = Field(default_factory=list)

    @classmethod
    def from_component_names(cls, component_names: list[str]) -> "PipelineConfig":
        return cls(components=[ComponentConfig(name=name) for name in component_names])
