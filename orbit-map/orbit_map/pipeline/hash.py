from __future__ import annotations

import hashlib
import json
import logging
from pathlib import Path

logger = logging.getLogger(__name__)


def compute_hashes(file_paths: list[Path], repo_path: Path) -> dict[str, str]:
    """Compute sha256 hex digests for each file, keyed by relative path string."""
    hashes: dict[str, str] = {}
    logger.debug("Hashing %d file(s) under %s", len(file_paths), repo_path)

    for rel_path in file_paths:
        full_path = repo_path / rel_path
        try:
            digest = hashlib.sha256(full_path.read_bytes()).hexdigest()
            hashes[str(rel_path)] = digest
        except OSError as exc:
            logger.warning("Skipping %s: %s", rel_path, exc)

    return hashes


def detect_changes(new_hashes: dict[str, str], output_dir: Path) -> list[str]:
    """Return list of relative paths that are new or changed vs. the cached hashes."""
    cache_path = output_dir.parent / "cache" / "hashes.json"
    logger.debug("Detecting changes with cache file %s", cache_path)

    if not cache_path.exists():
        logger.info("Hash cache not found; treating all files as changed")
        return sorted(new_hashes.keys())

    try:
        old_hashes: dict[str, str] = json.loads(cache_path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        logger.warning("Failed to read hash cache: %s", exc)
        return sorted(new_hashes.keys())

    changed = [
        path for path, digest in new_hashes.items() if old_hashes.get(path) != digest
    ]
    logger.debug("Detected %d changed path(s)", len(changed))
    return sorted(changed)


def save_hash_cache(hashes: dict[str, str], output_dir: Path) -> None:
    """Persist hashes to .orbit/cache/hashes.json."""
    cache_path = output_dir.parent / "cache" / "hashes.json"
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    logger.debug("Writing hash cache to %s", cache_path)
    cache_path.write_text(json.dumps(hashes, indent=2, sort_keys=True) + "\n")
