from __future__ import annotations

import os

from .base import BaseAgent

DEFAULT_PROVIDER = "openai"
DEFAULT_MODELS = {
    "anthropic": "claude-3-5-haiku-latest",
    "ollama": "llama3.2",
    "openai": "gpt-4o-mini",
}


def get_agent(provider: str | None = None, model: str | None = None) -> BaseAgent:
    provider_name = (
        (provider or os.getenv("ORBIT_AGENT_PROVIDER", DEFAULT_PROVIDER))
        .strip()
        .lower()
    )
    model_name = (
        model or os.getenv("ORBIT_AGENT_MODEL") or DEFAULT_MODELS.get(provider_name)
    )

    if not model_name:
        raise ValueError(f"No default model configured for provider '{provider_name}'")

    if provider_name == "openai":
        from .openai import OpenAIAgent

        return OpenAIAgent(model=model_name)
    if provider_name == "anthropic":
        from .anthropic import AnthropicAgent

        return AnthropicAgent(model=model_name)
    if provider_name == "ollama":
        from .ollama import OllamaAgent

        return OllamaAgent(model=model_name)

    supported = ", ".join(sorted(DEFAULT_MODELS))
    raise ValueError(
        f"Unsupported provider '{provider_name}'. Expected one of: {supported}"
    )
