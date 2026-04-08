from __future__ import annotations

from abc import ABC, abstractmethod


class BaseAgent(ABC):
    def __init__(self, model: str) -> None:
        self.model = model

    @abstractmethod
    def chat(self, system_prompt: str, user_message: str) -> str:
        """Return assistant text for a single system+user exchange."""
