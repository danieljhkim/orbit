"""Oracle dispatcher — reads a fixture YAML and grades a final message.

Supports three oracle kinds (exactly one per fixture):
    grep:  substring / substring-absent checks against the final message.
    cmd:   a shell command that must exit 0, run in the sandbox.
    judge: deferred to scripts/judge.py (Phase 3).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml


def load_fixture(path: str | Path) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def grade(fixture: dict, final_message: str, *, sandbox: str | None = None) -> tuple[str, str]:
    """Return (verdict, rationale). Verdict is 'pass' or 'fail'."""
    oracle = fixture.get("oracle", {})
    if "grep" in oracle:
        return _grade_grep(oracle["grep"], final_message)
    if "cmd" in oracle:
        return _grade_cmd(oracle["cmd"], sandbox=sandbox)
    if "judge" in oracle:
        return ("fail", "judge oracle not implemented until Phase 3 — run judge.py manually")
    return ("fail", f"fixture has no recognized oracle (keys: {list(oracle.keys())})")


def _grade_grep(spec: dict, message: str) -> tuple[str, str]:
    must = spec.get("must_include", []) or []
    must_not = spec.get("must_not_include", []) or []
    missing = [s for s in must if s not in message]
    forbidden = [s for s in must_not if s in message]
    if missing:
        return ("fail", f"missing required substring(s): {missing!r}")
    if forbidden:
        return ("fail", f"found forbidden substring(s): {forbidden!r}")
    return ("pass", f"all {len(must)} required substrings present, no forbidden hits")


def _grade_cmd(spec: dict, *, sandbox: str | None) -> tuple[str, str]:
    shell = spec["shell"]
    cwd = spec.get("cwd", sandbox)
    if cwd and "{sandbox}" in cwd:
        cwd = cwd.replace("{sandbox}", sandbox or ".")
    try:
        proc = subprocess.run(
            shell, shell=True, cwd=cwd, capture_output=True, text=True, timeout=600
        )
    except subprocess.TimeoutExpired:
        return ("fail", f"oracle command timed out after 600s: {shell!r}")
    if proc.returncode == 0:
        return ("pass", f"oracle command exited 0: {shell!r}")
    tail = (proc.stderr or proc.stdout or "")[-200:].strip()
    return ("fail", f"oracle command exited {proc.returncode}: {shell!r} | {tail}")
