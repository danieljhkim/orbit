#!/usr/bin/env python3
"""Keep embedded skills and activities portable across consumer workspaces."""

from __future__ import annotations

import re
import sys
import tempfile
from argparse import ArgumentParser
from pathlib import Path


SOURCE_PATH = "crates/..."
PRIVATE_NAMES = ("almanac", "dk-mac", "dk-server", "Constellation")
PERSONAL_NAMES = ("Daniel",)


def artifact_ids(content: str) -> list[str]:
    """Return concrete workspace-local artifact IDs, not placeholders."""
    boundary = r"(?<![A-Za-z0-9])"
    patterns = (
        rf"{boundary}ORB-\d+",
        rf"{boundary}ADR-\d+",
        rf"{boundary}L-\d{{3,}}",
        rf"{boundary}F\d{{4}}-\d{{2}}-\d{{3}}",
        rf"{boundary}T\d{{6,}}",
    )
    return [match.group() for pattern in patterns for match in re.finditer(pattern, content)]


def violations(content: str) -> list[str]:
    """Return all repository-portability violations in one embedded asset."""
    found: list[str] = []
    if "crates/" in content:
        found.append(f"Orbit source path `{SOURCE_PATH}`")

    for name in PRIVATE_NAMES:
        if name in content:
            found.append(f"private name `{name}`")
    for name in PERSONAL_NAMES:
        if name in content:
            found.append(f"personal name `{name}` (state the role, not the person)")

    if re.search(r'"model"\s*:\s*"codex"', content):
        found.append("hard-coded agent model `codex`")

    design_doc = re.search(r"(?<![A-Za-z0-9])\d_[a-z_]+\.md", content)
    if design_doc:
        found.append(f"fixed design-doc filename `{design_doc.group()}`")

    found.extend(f"workspace-local artifact id `{artifact_id}`" for artifact_id in artifact_ids(content))
    return found


def failures_under(label: str, root: Path) -> list[str]:
    failures: list[str] = []
    for path in sorted(candidate for candidate in root.rglob("*") if candidate.is_file()):
        relative = path.relative_to(root)
        for violation in violations(path.read_text(encoding="utf-8")):
            failures.append(f"{label}/{relative}: {violation}")
    return failures


def check_assets(root: Path) -> list[str]:
    return failures_under("skills", root / "skills") + failures_under("activities", root / "activities")


def verify_fixture_reporting() -> None:
    """Prove that the fast check emits a useful path and violation."""
    with tempfile.TemporaryDirectory(prefix="orbit-portability-") as temporary:
        assets = Path(temporary)
        fixture = assets / "skills" / "fixture.md"
        fixture.parent.mkdir()
        fixture.write_text("See ORB-10530 before handoff.\n", encoding="utf-8")
        (assets / "activities").mkdir()

        expected = "skills/fixture.md: workspace-local artifact id `ORB-10530`"
        failures = check_assets(assets)
        if failures != [expected]:
            raise RuntimeError(
                "temporary fixture did not report its relative path and violation: "
                f"expected {expected!r}, got {failures!r}"
            )


def main() -> int:
    parser = ArgumentParser()
    parser.add_argument(
        "--assets-root",
        type=Path,
        help="assets root to scan (used by the fixture regression check)",
    )
    arguments = parser.parse_args()
    repo_root = Path(__file__).resolve().parent.parent
    assets = arguments.assets_root or repo_root / "crates" / "orbit-core" / "assets"

    try:
        verify_fixture_reporting()
        failures = check_assets(assets)
    except (OSError, RuntimeError) as error:
        print(f"embedded-asset-portability: {error}", file=sys.stderr)
        return 2

    if failures:
        print(
            "embedded assets under crates/orbit-core/assets/{skills,activities}/ "
            "are not repository-agnostic:",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
