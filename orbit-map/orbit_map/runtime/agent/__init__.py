"""Agent provider runtime boundary.

Graph and deterministic context renderers should not import provider
implementations. Optional LLM-backed pipeline components may depend on this
runtime boundary when they explicitly need model calls.
"""

from .base import BaseAgent
from .factory import get_agent

__all__ = [
    "BaseAgent",
    "get_agent",
]
