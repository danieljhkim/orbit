from __future__ import annotations

from anthropic import Anthropic

from .base import BaseAgent


class AnthropicAgent(BaseAgent):
    def __init__(
        self, model: str, client: Anthropic | None = None, max_tokens: int = 4096
    ) -> None:
        super().__init__(model=model)
        self.client = client or Anthropic()
        self.max_tokens = max_tokens

    def chat(self, system_prompt: str, user_message: str) -> str:
        response = self.client.messages.create(
            model=self.model,
            system=system_prompt,
            messages=[{"role": "user", "content": user_message}],
            temperature=0,
            max_tokens=self.max_tokens,
        )
        parts = [
            block.text
            for block in response.content
            if getattr(block, "type", None) == "text"
        ]
        return "".join(parts)
