from __future__ import annotations

from pathlib import Path

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


def detect_language(file_path: str | Path) -> str:
    ext = Path(file_path).suffix.lower()
    return EXTENSION_MAP.get(ext, ext.lstrip(".") if ext else "unknown")
