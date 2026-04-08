from __future__ import annotations

from openai import OpenAI

from .base import BaseAgent


class OpenAIAgent(BaseAgent):
    def __init__(self, model: str, client: OpenAI | None = None) -> None:
        super().__init__(model=model)
        self.client = client or OpenAI()

    def chat(self, system_prompt: str, user_message: str) -> str:
        response = self.client.chat.completions.create(
            model=self.model,
            messages=[
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message},
            ],
            temperature=0,
        )
        content = response.choices[0].message.content
        return content if content is not None else ""
