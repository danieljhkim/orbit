from __future__ import annotations

import subprocess
from pathlib import Path

SKIP_DIRS: set[str] = {
    "node_modules",
    "__pycache__",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    ".egg-info",
}

SKIP_EXTENSIONS: set[str] = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".woff",
    ".woff2",
    ".ttf",
    ".eot",
    ".pdf",
    ".zip",
    ".tar",
    ".gz",
    ".lock",
    ".exe",
    ".dll",
    ".so",
    ".dylib",
}


def scan_repo(repo_path: Path) -> list[Path]:
    """Walk the repo and return all files that should be indexed as relative paths."""
    candidates: list[Path] = []

    for path in repo_path.rglob("*"):
        if not path.is_file():
            continue

        rel = path.relative_to(repo_path)

        # Skip hidden directories (any part starting with '.')
        if any(part.startswith(".") for part in rel.parts):
            continue

        # Skip common non-code directories
        if any(part in SKIP_DIRS for part in rel.parts):
            continue

        # Skip binary/non-text extensions
        if path.suffix.lower() in SKIP_EXTENSIONS:
            continue

        candidates.append(rel)

    ignored = _git_ignored_paths(repo_path, candidates)
    results = [path for path in candidates if path not in ignored]
    results.sort()
    return results


def _git_ignored_paths(repo_path: Path, paths: list[Path]) -> set[Path]:
    if not paths:
        return set()

    input_text = "".join(f"{path.as_posix()}\n" for path in paths)
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_path), "check-ignore", "--stdin"],
            input=input_text,
            capture_output=True,
            check=False,
            text=True,
        )
    except OSError:
        return set()

    if result.returncode not in {0, 1}:
        return set()

    return {Path(line) for line in result.stdout.splitlines() if line}
