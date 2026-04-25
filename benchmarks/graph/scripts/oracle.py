"""Oracle dispatcher — reads a fixture YAML and grades a final message.

Supports four oracle kinds (exactly one per fixture):
    grep:       substring / substring-absent checks against the final message.
    cmd:        a shell command that must exit 0, run in the sandbox.
    judge:      deferred to scripts/judge.py (Phase 3).
    structured: parse final message as `{"answer": [...], "excluded": [...]}`
                JSON and grade `answer` against ground truth as a set.
                v4 default. See `_grade_structured` for spec.
"""

from __future__ import annotations

import json
import re
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
    if "structured" in oracle:
        return _grade_structured(oracle["structured"], final_message)
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


def _grade_structured(spec: dict, message: str) -> tuple[str, str]:
    """Grade a structured-output oracle.

    Spec shape:
        item_kind:      str, informational  (file_path | symbol_name | ...)
        scoring:        'exact_set' | 'superset_ok' | 'subset_ok'
                        (default: 'exact_set')
        case_sensitive: bool, default False
        ground_truth:
            answer:               list[str], the expected items
            excluded_acceptable:  list[str], optional, informational only
        deny_list:      optional list of {pattern, reason} dicts. If any
                        pattern appears as a substring of any answer item,
                        the run fails regardless of set scoring.

    The agent's final message is parsed as a JSON object with shape
    `{"answer": [...], "excluded": [...]}`. The `answer` list is matched
    against `ground_truth.answer` as a set. The `excluded` list is
    informational and not graded.
    """
    answer = _extract_answer_list(message)
    if answer is None:
        return (
            "fail",
            "could not extract structured JSON answer; expected "
            '{"answer": [...], "excluded": [...]} in the final message',
        )

    expected = list(spec.get("ground_truth", {}).get("answer", []))
    case_sensitive = bool(spec.get("case_sensitive", False))
    scoring = spec.get("scoring", "exact_set")

    norm = (lambda s: s) if case_sensitive else (lambda s: s.lower())
    answer_set = {norm(s) for s in answer}
    expected_set = {norm(s) for s in expected}

    if scoring == "exact_set":
        if answer_set != expected_set:
            missing = sorted(expected_set - answer_set)
            extra = sorted(answer_set - expected_set)
            details = []
            if missing:
                details.append(f"missing {len(missing)}: {missing[:5]}")
            if extra:
                details.append(f"unexpected {len(extra)}: {extra[:5]}")
            return ("fail", "; ".join(details) or "set mismatch")
    elif scoring == "superset_ok":
        missing = sorted(expected_set - answer_set)
        if missing:
            return ("fail", f"missing {len(missing)}: {missing[:5]}")
    elif scoring == "subset_ok":
        extra = sorted(answer_set - expected_set)
        if extra:
            return ("fail", f"unexpected {len(extra)}: {extra[:5]}")
    else:
        return ("fail", f"unknown scoring mode: {scoring!r}")

    deny = spec.get("deny_list", []) or []
    forbidden_hits = []
    for entry in deny:
        pattern = entry.get("pattern", "")
        if not pattern:
            continue
        for item in answer:
            if pattern in str(item):
                forbidden_hits.append((pattern, item, entry.get("reason", "")))
    if forbidden_hits:
        sample = forbidden_hits[:3]
        return ("fail", f"deny_list violations ({len(forbidden_hits)}): {sample}")

    return (
        "pass",
        f"structured answer matches {scoring} of {len(expected)} expected items",
    )


# ---------------------------------------------------------------------------
# Helpers


_FENCE_RE = re.compile(r"```(?:json)?\s*(\{.*?\})\s*```", re.DOTALL)


def _extract_answer_list(message: str) -> list[str] | None:
    """Extract the 'answer' field from a structured JSON output.

    Tries, in order:
      1. The whole message as JSON.
      2. Each ```json ... ``` or ``` ... ``` fenced block.
      3. The last balanced {...} block in the message.
    Returns the answer list as `list[str]`, or None if no candidate parses
    to a dict with an `answer` list.
    """
    candidates: list[str] = [message.strip()]

    for match in _FENCE_RE.finditer(message):
        candidates.append(match.group(1))

    last_brace = message.rfind("}")
    if last_brace >= 0:
        depth = 0
        for i in range(last_brace, -1, -1):
            ch = message[i]
            if ch == "}":
                depth += 1
            elif ch == "{":
                depth -= 1
                if depth == 0:
                    candidates.append(message[i : last_brace + 1])
                    break

    for c in candidates:
        try:
            obj = json.loads(c)
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict) and isinstance(obj.get("answer"), list):
            return [str(x) for x in obj["answer"]]

    return None
