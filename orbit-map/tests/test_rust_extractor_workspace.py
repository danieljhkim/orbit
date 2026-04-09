import json
from pathlib import Path

import pytest

from orbit_map.graph.extraction.base import GraphExtractionInput, source_hash
from orbit_map.graph.extraction.rust import RustGraphExtractor


REPO_ROOT = Path(__file__).resolve().parents[2]
REPORT_PATH = REPO_ROOT / "orbit-map" / "tmp" / "rust_extractor_report.json"


def _meaningful_source_lines(source: str) -> list[str]:
    lines: list[str] = []
    for raw_line in source.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("//"):
            continue
        lines.append(line)
    return lines


def _is_reexport_only_module(source: str) -> bool:
    lines = _meaningful_source_lines(source)
    return bool(lines) and all(line.startswith("pub use ") for line in lines)


@pytest.mark.slow
def test_rust_extractor_scans_orbit_workspace_without_exceptions():
    extractor = RustGraphExtractor()
    rust_files = sorted((REPO_ROOT / "crates").glob("orbit-*/src/**/*.rs"))

    raised_files = []
    zero_leaf_files = []
    intentional_empty_files = []
    reexport_only_files = []
    unexpected_zero_leaf_files = []
    leaf_count_by_file = {}
    total_leaves = 0
    total_pub_items = 0
    total_impl_methods = 0

    for path in rust_files:
        source = path.read_text(encoding="utf-8")
        relative_path = str(path.relative_to(REPO_ROOT))

        try:
            result = extractor.extract(
                GraphExtractionInput(
                    path=relative_path,
                    source=source,
                    file_id=f"file:{relative_path}",
                    file_hash=source_hash(source),
                )
            )
        except Exception as exc:  # pragma: no cover - failure path is asserted below
            raised_files.append(
                {
                    "path": relative_path,
                    "error_type": type(exc).__name__,
                    "message": str(exc),
                }
            )
            continue

        leaf_count = len(result.leaves)
        leaf_count_by_file[relative_path] = leaf_count
        total_leaves += leaf_count
        total_pub_items += len(result.exports)
        total_impl_methods += sum(1 for leaf in result.leaves if leaf.kind == "method")

        if leaf_count == 0:
            zero_leaf_files.append(relative_path)
            if not _meaningful_source_lines(source):
                intentional_empty_files.append(relative_path)
            elif _is_reexport_only_module(source):
                reexport_only_files.append(relative_path)
            else:
                unexpected_zero_leaf_files.append(relative_path)

    files_scanned = len(rust_files)
    files_with_leaves = files_scanned - len(zero_leaf_files)
    report = {
        "files_scanned": files_scanned,
        "files_with_leaves": files_with_leaves,
        "files_with_leaves_fraction": files_with_leaves / files_scanned if files_scanned else 0.0,
        "files_with_zero_leaves": len(zero_leaf_files),
        "zero_leaf_files": zero_leaf_files,
        "intentional_empty_files": intentional_empty_files,
        "reexport_only_files": reexport_only_files,
        "unexpected_zero_leaf_files": unexpected_zero_leaf_files,
        "leaf_count_by_file": leaf_count_by_file,
        "total_leaves": total_leaves,
        "total_pub_items": total_pub_items,
        "total_impl_methods": total_impl_methods,
        "raised_files": raised_files,
    }

    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    assert raised_files == []
    assert report["files_scanned"] > 0
    assert report["files_with_leaves_fraction"] >= 0
    assert unexpected_zero_leaf_files == []
