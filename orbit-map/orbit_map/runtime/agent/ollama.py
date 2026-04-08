from __future__ import annotations

import json
import os
from urllib import request

from .base import BaseAgent


class OllamaAgent(BaseAgent):
    def __init__(
        self,
        model: str,
        base_url: str | None = None,
        timeout_seconds: float = 120.0,
    ) -> None:
        super().__init__(model=model)
        self.base_url = (
            base_url or os.getenv("OLLAMA_BASE_URL", "http://localhost:11434")
        ).rstrip("/")
        self.timeout_seconds = timeout_seconds

    def chat(self, system_prompt: str, user_message: str) -> str:
        payload = json.dumps(
            {
                "model": self.model,
                "stream": False,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_message},
                ],
                "options": {"temperature": 0},
            }
        ).encode("utf-8")
        req = request.Request(
            f"{self.base_url}/api/chat",
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with request.urlopen(req, timeout=self.timeout_seconds) as response:
            raw = response.read().decode("utf-8")

        parsed = json.loads(raw)
        content = parsed.get("message", {}).get("content", "")
        return content if isinstance(content, str) else ""
