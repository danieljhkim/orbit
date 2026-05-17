#!/usr/bin/env python3
"""One-shot migration from model-string identity to AgentFamily identity.

The migration is intentionally conservative: discovery is separate from apply,
dry-run is the default, and every apply starts by copying the whole `.orbit`
directory to a timestamped backup. Slot recovery for historical planning-duel
artifacts prefers explicit role metadata; if no role file exists, it falls back
to winner metadata and then to deterministic filename order for complete
two-planner artifact sets. A single orphan artifact is left untouched because
there is no safe way to know whether it was planner_a or planner_b.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import re
import shutil
import sqlite3
import sys
import tempfile
import time
from collections import defaultdict
from pathlib import Path
from typing import Any


MIGRATION_NAME = "family_identity_v1"

FAMILIES = {"codex", "claude", "gemini", "grok"}
ACTORS = {"human", "admin", "system"}
SLOTS = ("planner_a", "planner_b", "arbiter")

LINEUP_KEYS = {
    "resolved_crew",
    "planner_model",
    "implementer_model",
    "reviewer_model",
}
SQL_SKIP_COLUMNS = {
    "model_id",
    "embedding_model",
    "provider_model",
    "object_hash",
    "hash",
    "path",
    "location",
    "language",
}
IDENTITY_KEYS = {
    "agent",
    "agent_identity",
    "actor",
    "author",
    "by",
    "created_by",
    "implemented_by",
    "planned_by",
    "reviewed_by",
    "reviewer_by",
    "model",
    "role",
    "winner_agent_cli",
}
FORBIDDEN_SCOREBOARD_TOKENS = (
    "grok-4",
    "claude-opus-4-7",
    "gpt-5.5",
    "opus-4.7",
    "pro",
    "grok-build",
)


@dataclasses.dataclass(frozen=True)
class Change:
    kind: str
    target: str
    old: str
    new: str

    def render(self) -> str:
        return f"{self.kind}: {self.target}: {self.old!r} -> {self.new!r}"


@dataclasses.dataclass(frozen=True)
class SqlUpdate:
    db_path: Path
    table: str
    column: str
    rowid: int
    old: str
    new: str


@dataclasses.dataclass(frozen=True)
class SqlAggregate:
    db_path: Path
    table: str
    columns: tuple[str, ...]
    rows: tuple[tuple[Any, ...], ...]
    old_row_count: int
    new_row_count: int
    reason: str


def infer_agent_family_from_model(model: str) -> str | None:
    value = model.strip().lower()
    if not value:
        return None
    if value == "pro":
        return "gemini"
    if value.startswith("gpt-") or value.startswith("o1") or value.startswith("o3"):
        return "codex"
    if value.startswith("claude-") or value.startswith("opus") or value.startswith("sonnet"):
        return "claude"
    if value.startswith("gemini-"):
        return "gemini"
    if value.startswith("grok-") or value.startswith("grok3"):
        return "grok"
    return None


def canonical_identity(value: Any, sibling_family: str | None = None) -> str | None:
    if not isinstance(value, str):
        return None
    raw = value.strip()
    if not raw:
        return None
    lower = raw.lower()
    if lower in FAMILIES:
        return lower
    if lower in ACTORS:
        return lower
    family = infer_agent_family_from_model(raw)
    if family:
        return family
    if " / " in raw:
        left, right = (part.strip() for part in raw.rsplit(" / ", 1))
        left_lower = left.lower()
        if left_lower in FAMILIES:
            return left_lower
        family = infer_agent_family_from_model(right)
        if family:
            return family
    return None


def identity_key(key: str) -> bool:
    lower = key.lower()
    if lower in LINEUP_KEYS or lower in SQL_SKIP_COLUMNS:
        return False
    if lower in IDENTITY_KEYS:
        return True
    if lower.endswith("_by") or lower.endswith("_agent") or lower.endswith("_identity"):
        return True
    return False


def quoted_identifier(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def compact_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def write_atomic(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", dir=str(path.parent), delete=False
    ) as handle:
        handle.write(content)
        temp_name = handle.name
    os.replace(temp_name, path)


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


class Migration:
    def __init__(self, orbit_dir: Path, apply: bool, verbose: bool) -> None:
        self.orbit_dir = orbit_dir.resolve()
        self.apply = apply
        self.verbose = verbose
        self.changes: list[Change] = []
        self.file_writes: dict[Path, str] = {}
        self.renames: list[tuple[Path, Path]] = []
        self.sqlite_updates: list[SqlUpdate] = []
        self.sqlite_aggregates: list[SqlAggregate] = []
        self.sqlite_markers: set[Path] = set()
        self.skips: list[str] = []
        self.discovery: list[str] = []

    def marker_path(self, store: str) -> Path:
        return self.orbit_dir / f".family_identity_{store}.migrated"

    def store_migrated(self, store: str) -> bool:
        return self.marker_path(store).exists()

    def add_store_marker(self, store: str) -> None:
        marker = self.marker_path(store)
        if marker.exists():
            return
        content = f"{MIGRATION_NAME}\n"
        self.changes.append(Change("MARKER", rel(marker), "", MIGRATION_NAME))
        self.file_writes[marker] = content

    def collect(self) -> None:
        self.collect_file_store("tasks", self.orbit_dir / "tasks")
        self.collect_file_store("frictions", self.orbit_dir / "frictions")
        self.collect_file_store("job_runs", self.orbit_dir / "state" / "job-runs")
        self.collect_file_store("audit", self.orbit_dir / "state" / "audit")
        self.collect_planning_duel_artifacts()
        self.collect_sqlite_stores()
        self.collect_scoreboards()

    def collect_file_store(self, store: str, root: Path) -> None:
        if not root.exists():
            return
        for path in iter_files_following_symlink_dirs(root):
            if not path.is_file() or path.name.startswith("."):
                continue
            if path.suffix == ".json":
                self.collect_json_file(path)
            elif path.suffix == ".jsonl":
                self.collect_jsonl_file(path)
            elif path.suffix in {".yaml", ".yml"}:
                self.collect_identity_text_file(path, frontmatter_only=False)
            elif path.suffix == ".md":
                self.collect_identity_text_file(path, frontmatter_only=True)
        if not self.store_migrated(store):
            self.add_store_marker(store)

    def collect_json_file(self, path: Path) -> None:
        try:
            data = json.loads(read_text(path))
        except (OSError, json.JSONDecodeError):
            return
        new_data = self.transform_json(data, path, ())
        if new_data != data:
            self.file_writes[path] = json.dumps(new_data, indent=2, sort_keys=True) + "\n"

    def collect_jsonl_file(self, path: Path) -> None:
        try:
            lines = read_text(path).splitlines()
        except OSError:
            return
        changed = False
        new_lines: list[str] = []
        for index, line in enumerate(lines, start=1):
            if not line.strip():
                new_lines.append(line)
                continue
            try:
                data = json.loads(line)
            except json.JSONDecodeError:
                new_lines.append(line)
                continue
            new_data = self.transform_json(data, path, (str(index),))
            changed = changed or new_data != data
            new_lines.append(json.dumps(new_data, sort_keys=True, separators=(",", ":")))
        if changed:
            self.file_writes[path] = "\n".join(new_lines) + "\n"

    def collect_identity_text_file(self, path: Path, frontmatter_only: bool) -> None:
        try:
            lines = read_text(path).splitlines(keepends=True)
        except OSError:
            return
        if frontmatter_only and (not lines or lines[0].strip() != "---"):
            return
        in_frontmatter = not frontmatter_only
        changed = False
        new_lines: list[str] = []
        scalar_pattern = re.compile(
            r"^([^\S\r\n]*)([A-Za-z_][A-Za-z0-9_-]*):[^\S\r\n]*([^\r\n]*?)[^\S\r\n]*(\r?\n?)$"
        )
        for index, line in enumerate(lines):
            if frontmatter_only and index == 0:
                new_lines.append(line)
                in_frontmatter = True
                continue
            if frontmatter_only and in_frontmatter and line.strip() == "---":
                new_lines.append(line)
                in_frontmatter = False
                continue
            if not in_frontmatter:
                new_lines.append(line)
                continue
            match = scalar_pattern.match(line)
            if not match:
                new_lines.append(line)
                continue
            indent, key, raw_value, newline = match.groups()
            if not identity_key(key):
                new_lines.append(line)
                continue
            value = raw_value.strip().strip('"').strip("'")
            new_value = canonical_identity(value)
            if new_value and new_value != value:
                self.changes.append(Change("TEXT", f"{rel(path)}:{key}", value, new_value))
                new_lines.append(f"{indent}{key}: {new_value}{newline}")
                changed = True
            else:
                new_lines.append(line)
        if changed:
            self.file_writes[path] = "".join(new_lines)

    def transform_json(self, value: Any, path: Path, pointer: tuple[str, ...]) -> Any:
        if isinstance(value, dict):
            sibling_family = canonical_identity(value.get("agent"))
            result: dict[str, Any] = {}
            for key, child in value.items():
                child_pointer = pointer + (key,)
                if key.lower() in LINEUP_KEYS or key.lower() in SQL_SKIP_COLUMNS:
                    result[key] = child
                    continue
                if isinstance(child, (dict, list)):
                    result[key] = self.transform_json(child, path, child_pointer)
                    continue
                if identity_key(key):
                    new_value = canonical_identity(
                        child,
                        sibling_family=sibling_family if key.lower() == "model" else None,
                    )
                    if new_value and new_value != child:
                        self.changes.append(
                            Change("JSON", f"{rel(path)}:{'/'.join(child_pointer)}", str(child), new_value)
                        )
                        result[key] = new_value
                        continue
                result[key] = child
            return result
        if isinstance(value, list):
            return [
                self.transform_json(child, path, pointer + (str(index),))
                for index, child in enumerate(value)
            ]
        return value

    def collect_planning_duel_artifacts(self) -> None:
        tasks = self.orbit_dir / "tasks"
        if not tasks.exists():
            return
        for duel_dir in iter_planning_duel_dirs(tasks):
            if not duel_dir.is_dir():
                continue
            self.collect_one_duel_dir(duel_dir)
        if not self.store_migrated("planning_duel_artifacts"):
            self.add_store_marker("planning_duel_artifacts")

    def collect_one_duel_dir(self, duel_dir: Path) -> None:
        task_id = task_id_for_duel_dir(duel_dir)
        role_slots = load_role_slots(duel_dir)
        winner = load_json_if_exists(duel_dir / "winner.json")
        manifest_path = duel_dir.parent.parent / "manifest.yaml"
        manifest_md_files = load_manifest_md_files(manifest_path, duel_dir)
        md_files = sorted(path for path in duel_dir.glob("*.md") if path.name != "winner.json")
        rename_map: dict[str, str] = {}
        projected_contents: dict[str, str] = {}
        projected_created_by: dict[str, str] = {}
        for path in md_files:
            try:
                content = read_text(path)
            except OSError:
                continue
            lines = content.splitlines(keepends=True)
            if not lines:
                continue
            parsed = parse_legacy_signature(lines[0].strip())
            if not parsed:
                continue
            agent, model = parsed
            family = canonical_identity(agent) or canonical_identity(model)
            if not family or family not in FAMILIES:
                continue
            slot = determine_slot(path, family, role_slots, winner, md_files, manifest_md_files)
            if not slot:
                message = f"SKIP: {task_id} {path.name}: slot undecidable"
                self.skips.append(message)
                continue
            new_first = f"*authored by: {family} / {slot}*\n"
            if lines[0] != new_first:
                self.changes.append(Change("DUEL", f"{rel(path)}:signature", lines[0].strip(), new_first.strip()))
                lines[0] = new_first
                self.file_writes[path] = "".join(lines)
            target = path.with_name(f"{slot}.md")
            new_rel = f"planning-duel/{target.name}"
            projected_contents[new_rel] = "".join(lines)
            projected_created_by[new_rel] = family
            if target != path and not target.exists():
                self.changes.append(Change("RENAME", rel(path), rel(path), rel(target)))
                self.renames.append((path, target))
                rename_map[f"planning-duel/{path.name}"] = new_rel
        winner_path = duel_dir / "winner.json"
        if isinstance(winner, dict):
            new_winner = dict(winner)
            winner_family = canonical_identity(
                new_winner.get("winner_agent_cli")
            ) or canonical_identity(new_winner.get("winner_model"))
            if winner_family:
                if new_winner.get("winner_family") != winner_family:
                    self.changes.append(
                        Change("DUEL", f"{rel(winner_path)}:winner_family", str(new_winner.get("winner_family")), winner_family)
                    )
                    new_winner["winner_family"] = winner_family
            arbiter_family = canonical_identity(new_winner.get("arbiter_family")) or canonical_identity(
                new_winner.get("arbiter_agent_cli")
            ) or canonical_identity(new_winner.get("arbiter_model"))
            if arbiter_family:
                if new_winner.get("arbiter_family") != arbiter_family:
                    self.changes.append(
                        Change("DUEL", f"{rel(winner_path)}:arbiter_family", str(new_winner.get("arbiter_family")), arbiter_family)
                    )
                    new_winner["arbiter_family"] = arbiter_family
            if "winner_agent_cli" in new_winner:
                self.changes.append(Change("DUEL", f"{rel(winner_path)}:winner_agent_cli", str(new_winner["winner_agent_cli"]), "<removed>"))
                del new_winner["winner_agent_cli"]
            if "winner_model" in new_winner:
                self.changes.append(Change("DUEL", f"{rel(winner_path)}:winner_model", str(new_winner["winner_model"]), "<removed>"))
                del new_winner["winner_model"]
            if "arbiter_agent_cli" in new_winner:
                self.changes.append(Change("DUEL", f"{rel(winner_path)}:arbiter_agent_cli", str(new_winner["arbiter_agent_cli"]), "<removed>"))
                del new_winner["arbiter_agent_cli"]
            if "arbiter_model" in new_winner:
                self.changes.append(Change("DUEL", f"{rel(winner_path)}:arbiter_model", str(new_winner["arbiter_model"]), "<removed>"))
                del new_winner["arbiter_model"]
            artifact_path = new_winner.get("artifact_path")
            if isinstance(artifact_path, str) and artifact_path in rename_map:
                self.changes.append(Change("DUEL", f"{rel(winner_path)}:artifact_path", artifact_path, rename_map[artifact_path]))
                new_winner["artifact_path"] = rename_map[artifact_path]
                artifact_path = new_winner["artifact_path"]
            if isinstance(artifact_path, str) and Path(artifact_path).stem in {"planner_a", "planner_b"}:
                winner_slot = Path(artifact_path).stem
                if new_winner.get("winner_slot") != winner_slot:
                    self.changes.append(
                        Change("DUEL", f"{rel(winner_path)}:winner_slot", str(new_winner.get("winner_slot")), winner_slot)
                    )
                    new_winner["winner_slot"] = winner_slot
            if new_winner != winner:
                winner_content = json.dumps(new_winner, indent=2, sort_keys=True) + "\n"
                self.file_writes[winner_path] = winner_content
                projected_contents["planning-duel/winner.json"] = winner_content
                if arbiter_family:
                    projected_created_by["planning-duel/winner.json"] = arbiter_family

        self.collect_manifest_update(
            manifest_path,
            rename_map,
            projected_contents,
            projected_created_by,
        )

    def collect_manifest_update(
        self,
        manifest_path: Path,
        rename_map: dict[str, str],
        projected_contents: dict[str, str],
        projected_created_by: dict[str, str],
    ) -> None:
        if not manifest_path.exists():
            return
        try:
            lines = read_text(manifest_path).splitlines(keepends=True)
        except OSError:
            return
        changed = False
        current_path: str | None = None
        new_lines: list[str] = []
        for line in lines:
            path_match = re.match(r"^(- path:\s*)(.+?)(\s*)(\r?\n?)$", line)
            if path_match:
                prefix, old_path, suffix, newline = path_match.groups()
                current_path = rename_map.get(old_path, old_path)
                if current_path != old_path:
                    changed = True
                    self.changes.append(Change("MANIFEST", f"{rel(manifest_path)}:path", old_path, current_path))
                new_lines.append(f"{prefix}{current_path}{suffix}{newline}")
                continue
            if current_path and line.startswith("  blob: "):
                desired = f"files/{current_path}"
                old = line.split(": ", 1)[1].strip()
                if current_path in projected_contents or old != desired:
                    if old != desired:
                        changed = True
                        self.changes.append(Change("MANIFEST", f"{rel(manifest_path)}:blob", old, desired))
                    new_lines.append(f"  blob: {desired}{line_newline(line)}")
                    continue
            if current_path and line.startswith("  sha256: ") and current_path in projected_contents:
                desired = hashlib_sha256(projected_contents[current_path])
                old = line.split(": ", 1)[1].strip()
                if old != desired:
                    changed = True
                    self.changes.append(Change("MANIFEST", f"{rel(manifest_path)}:sha256", old, desired))
                new_lines.append(f"  sha256: {desired}{line_newline(line)}")
                continue
            if current_path and line.startswith("  size_bytes: ") and current_path in projected_contents:
                desired = str(len(projected_contents[current_path].encode("utf-8")))
                old = line.split(": ", 1)[1].strip()
                if old != desired:
                    changed = True
                    self.changes.append(Change("MANIFEST", f"{rel(manifest_path)}:size_bytes", old, desired))
                new_lines.append(f"  size_bytes: {desired}{line_newline(line)}")
                continue
            if current_path and line.startswith("  created_by: ") and current_path in projected_created_by:
                desired = projected_created_by[current_path]
                old = line.split(": ", 1)[1].strip()
                if old != desired:
                    changed = True
                    self.changes.append(Change("MANIFEST", f"{rel(manifest_path)}:created_by", old, desired))
                new_lines.append(f"  created_by: {desired}{line_newline(line)}")
                continue
            new_lines.append(line)
        if changed:
            self.file_writes[manifest_path] = "".join(new_lines)

    def collect_sqlite_stores(self) -> None:
        for db_path in (
            self.orbit_dir / "orbit.db",
            self.orbit_dir / "state" / "orbit.sqlite",
            self.orbit_dir / "state" / "semantic.db",
            self.orbit_dir / "knowledge" / "graph" / "graph_index.sqlite",
        ):
            if db_path.exists():
                self.collect_sqlite_store(db_path)

    def collect_sqlite_store(self, db_path: Path) -> None:
        with sqlite3.connect(db_path) as conn:
            if sqlite_migration_applied(conn):
                return
            tables = [
                row[0]
                for row in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
                )
            ]
            for table in tables:
                if table == "_migrations":
                    continue
                self.collect_sqlite_table(conn, db_path, table)
            self.sqlite_markers.add(db_path)
            self.changes.append(Change("SQLITE-MARKER", rel(db_path), "", MIGRATION_NAME))

    def collect_sqlite_table(self, conn: sqlite3.Connection, db_path: Path, table: str) -> None:
        columns_info = list(conn.execute(f"PRAGMA table_info({quoted_identifier(table)})"))
        text_columns = [row for row in columns_info if sqlite_identity_column(row[1], row[2])]
        candidate_columns: list[str] = []
        for _, name, _, _, _, _ in text_columns:
            samples = sample_column_values(conn, table, name)
            if any(canonical_identity(sample) for sample in samples):
                candidate_columns.append(name)
                self.discovery.append(
                    f"{rel(db_path)}:{table}:{name} samples={samples[:5]}"
                )
        if not candidate_columns:
            return
        aggregate = build_aggregate_if_needed(conn, db_path, table, columns_info, candidate_columns)
        if aggregate:
            self.sqlite_aggregates.append(aggregate)
            self.changes.append(
                Change(
                    "SQLITE",
                    f"{rel(db_path)}:{table}",
                    f"{aggregate.old_row_count} rows",
                    f"{aggregate.new_row_count} aggregated rows ({aggregate.reason})",
                )
            )
            return
        table_q = quoted_identifier(table)
        for column in candidate_columns:
            column_q = quoted_identifier(column)
            try:
                rows = list(conn.execute(f"SELECT rowid, {column_q} FROM {table_q} WHERE {column_q} IS NOT NULL"))
            except sqlite3.DatabaseError:
                continue
            for rowid, old in rows:
                new = canonical_identity(old)
                if new and new != old:
                    update = SqlUpdate(db_path, table, column, int(rowid), old, new)
                    self.sqlite_updates.append(update)
                    self.changes.append(
                        Change(
                            "SQLITE",
                            f"{rel(db_path)}:{table}:{column}:rowid={rowid}",
                            str(old),
                            new,
                        )
                    )

    def collect_scoreboards(self) -> None:
        root = self.orbit_dir / "state" / "scoreboard"
        if not root.exists():
            return
        marker_exists = self.store_migrated("scoreboard")
        for path in sorted(root.glob("*.json")):
            data = load_json_if_exists(path)
            if data is None:
                continue
            new_data = rewrite_scoreboard(path.name, data, self.orbit_dir)
            findings = scoreboard_forbidden_findings(new_data)
            if findings:
                for finding in findings:
                    self.skips.append(f"SCOREBOARD-CHECK: {path.name}: {finding}")
            if new_data != data:
                self.changes.append(Change("SCOREBOARD", rel(path), "<legacy scoreboard>", "<family-keyed scoreboard>"))
                self.file_writes[path] = json.dumps(new_data, indent=2, sort_keys=True) + "\n"
        if not marker_exists:
            self.add_store_marker("scoreboard")

    def print_report(self) -> None:
        mode = "APPLY" if self.apply else "DRY-RUN"
        print(f"{mode}: {self.orbit_dir}")
        if self.discovery:
            print("SQLite discovery:")
            for line in self.discovery:
                print(f"  {line}")
        for skip in self.skips:
            print(skip)
        for change in self.changes:
            print(change.render())
        print(f"Summary: {len(self.changes)} changes")

    def apply_changes(self) -> None:
        if not self.apply or not self.changes:
            return
        backup_path = backup_orbit_dir(self.orbit_dir)
        print(f"Backup: {backup_path}")
        for path, content in self.file_writes.items():
            write_atomic(path, content)
        for old, new in self.renames:
            if old.exists() and not new.exists():
                new.parent.mkdir(parents=True, exist_ok=True)
                old.rename(new)
        self.apply_sqlite_changes()

    def apply_sqlite_changes(self) -> None:
        dbs = sorted(
            {update.db_path for update in self.sqlite_updates}
            | {aggregate.db_path for aggregate in self.sqlite_aggregates}
            | self.sqlite_markers
        )
        for db_path in dbs:
            with sqlite3.connect(db_path) as conn:
                conn.execute("BEGIN")
                try:
                    for aggregate in self.sqlite_aggregates:
                        if aggregate.db_path == db_path:
                            apply_aggregate(conn, aggregate)
                    for update in self.sqlite_updates:
                        if update.db_path == db_path:
                            conn.execute(
                                f"UPDATE {quoted_identifier(update.table)} SET {quoted_identifier(update.column)} = ? WHERE rowid = ?",
                                (update.new, update.rowid),
                            )
                    ensure_sqlite_migration_marker(conn)
                except Exception:
                    conn.rollback()
                    raise
                else:
                    conn.commit()


def sqlite_identity_column(name: str, declared_type: str | None) -> bool:
    lower = name.lower()
    if lower in SQL_SKIP_COLUMNS or lower in LINEUP_KEYS:
        return False
    if not identity_key(lower):
        return False
    type_lower = (declared_type or "").lower()
    return not type_lower or any(token in type_lower for token in ("text", "char", "clob", "varchar"))


def sample_column_values(conn: sqlite3.Connection, table: str, column: str) -> list[str]:
    try:
        rows = conn.execute(
            f"SELECT DISTINCT {quoted_identifier(column)} FROM {quoted_identifier(table)} "
            f"WHERE {quoted_identifier(column)} IS NOT NULL LIMIT 20"
        )
        return [str(row[0]) for row in rows]
    except sqlite3.DatabaseError:
        return []


def sqlite_migration_applied(conn: sqlite3.Connection) -> bool:
    try:
        row = conn.execute(
            "SELECT 1 FROM _migrations WHERE name = ? LIMIT 1", (MIGRATION_NAME,)
        ).fetchone()
        return row is not None
    except sqlite3.DatabaseError:
        return False


def ensure_sqlite_migration_marker(conn: sqlite3.Connection) -> None:
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _migrations (name TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)"
    )
    conn.execute(
        "INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?, ?)",
        (MIGRATION_NAME, int(time.time())),
    )


def unique_column_sets(conn: sqlite3.Connection, table: str, columns_info: list[tuple[Any, ...]]) -> list[tuple[str, ...]]:
    sets: list[tuple[str, ...]] = []
    pk_cols = tuple(row[1] for row in sorted(columns_info, key=lambda item: item[5]) if row_pk(row))
    if pk_cols:
        sets.append(pk_cols)
    for index_row in conn.execute(f"PRAGMA index_list({quoted_identifier(table)})"):
        if not index_row[2]:
            continue
        index_name = index_row[1]
        cols = tuple(
            row[2]
            for row in conn.execute(f"PRAGMA index_info({quoted_identifier(index_name)})")
            if row[2]
        )
        if cols:
            sets.append(cols)
    return sets


def row_pk(row: tuple[Any, ...]) -> bool:
    return int(row[5] or 0) > 0


def build_aggregate_if_needed(
    conn: sqlite3.Connection,
    db_path: Path,
    table: str,
    columns_info: list[tuple[Any, ...]],
    candidate_columns: list[str],
) -> SqlAggregate | None:
    unique_sets = unique_column_sets(conn, table, columns_info)
    relevant_sets = [cols for cols in unique_sets if any(col in candidate_columns for col in cols)]
    if not relevant_sets:
        return None
    columns = [row[1] for row in columns_info]
    table_q = quoted_identifier(table)
    rows = list(conn.execute(f"SELECT {', '.join(quoted_identifier(col) for col in columns)} FROM {table_q}"))
    if not rows:
        return None
    for key_cols in relevant_sets:
        groups: dict[tuple[Any, ...], list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            item = dict(zip(columns, row))
            key: list[Any] = []
            for col in key_cols:
                value = item[col]
                key.append(canonical_identity(value) if col in candidate_columns else value)
            groups[tuple(key)].append(item)
        if not any(len(items) > 1 for items in groups.values()):
            continue
        numeric_cols = [
            row[1]
            for row in columns_info
            if row[1] not in key_cols and sqlite_numeric_column(row[2])
        ]
        aggregated_rows: list[tuple[Any, ...]] = []
        for key, items in sorted(groups.items(), key=lambda pair: str(pair[0])):
            output: dict[str, Any] = {}
            for col in columns:
                if col in key_cols:
                    output[col] = key[list(key_cols).index(col)]
                elif col in numeric_cols:
                    output[col] = sum(value_as_number(item.get(col)) for item in items)
                elif col in candidate_columns:
                    value = items[0].get(col)
                    output[col] = canonical_identity(value) or value
                else:
                    output[col] = next((item.get(col) for item in items if item.get(col) is not None), None)
            aggregated_rows.append(tuple(output[col] for col in columns))
        return SqlAggregate(
            db_path=db_path,
            table=table,
            columns=tuple(columns),
            rows=tuple(aggregated_rows),
            old_row_count=len(rows),
            new_row_count=len(aggregated_rows),
            reason=f"collapsed unique key {','.join(key_cols)}",
        )
    return None


def sqlite_numeric_column(declared_type: str | None) -> bool:
    lower = (declared_type or "").lower()
    return any(token in lower for token in ("int", "real", "numeric", "decimal", "double", "float"))


def value_as_number(value: Any) -> int | float:
    if isinstance(value, (int, float)):
        return value
    if value is None:
        return 0
    try:
        return int(value)
    except (TypeError, ValueError):
        try:
            return float(value)
        except (TypeError, ValueError):
            return 0


def apply_aggregate(conn: sqlite3.Connection, aggregate: SqlAggregate) -> None:
    table = quoted_identifier(aggregate.table)
    conn.execute(f"DELETE FROM {table}")
    placeholders = ", ".join("?" for _ in aggregate.columns)
    columns = ", ".join(quoted_identifier(col) for col in aggregate.columns)
    conn.executemany(
        f"INSERT INTO {table} ({columns}) VALUES ({placeholders})",
        list(aggregate.rows),
    )


def parse_legacy_signature(line: str) -> tuple[str, str] | None:
    match = re.match(r"^\*authored by:\s*([^/]+?)\s*/\s*([^*]+?)\s*\*$", line)
    if not match:
        return None
    return match.group(1).strip(), match.group(2).strip()


def task_id_for_duel_dir(duel_dir: Path) -> str:
    for part in duel_dir.parts:
        if re.match(r"^ORB-\d+$", part):
            return part
    return "<unknown-task>"


def iter_files_following_symlink_dirs(root: Path) -> list[Path]:
    files: list[Path] = []
    seen_dirs: set[Path] = set()

    def walk(directory: Path) -> None:
        try:
            resolved = directory.resolve()
        except OSError:
            return
        if resolved in seen_dirs:
            return
        seen_dirs.add(resolved)
        try:
            children = sorted(directory.iterdir(), key=lambda path: path.name)
        except OSError:
            return
        for child in children:
            if child.name.startswith("."):
                continue
            if child.is_dir():
                walk(child)
            elif child.is_file():
                files.append(child)

    walk(root)
    return files


def iter_planning_duel_dirs(tasks_root: Path) -> list[Path]:
    duel_dirs: list[Path] = []
    try:
        task_entries = sorted(tasks_root.iterdir(), key=lambda path: path.name)
    except OSError:
        return duel_dirs
    for task_entry in task_entries:
        if task_entry.name.startswith(".") or not task_entry.is_dir():
            continue
        duel_dir = task_entry / "artifacts" / "files" / "planning-duel"
        if duel_dir.is_dir():
            duel_dirs.append(duel_dir)
    return duel_dirs


def line_newline(line: str) -> str:
    return "\n" if line.endswith("\n") else ""


def hashlib_sha256(content: str) -> str:
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def load_json_if_exists(path: Path) -> Any | None:
    if not path.exists():
        return None
    try:
        return json.loads(read_text(path))
    except (OSError, json.JSONDecodeError):
        return None


def load_role_slots(duel_dir: Path) -> dict[str, str]:
    data = load_json_if_exists(duel_dir / "planning_duel_roles.json")
    if not isinstance(data, dict):
        return {}
    roles = data.get("planning_duel_roles") if isinstance(data.get("planning_duel_roles"), dict) else data
    slots: dict[str, str] = {}
    for slot in SLOTS:
        role = roles.get(slot) if isinstance(roles, dict) else None
        if not isinstance(role, dict):
            continue
        family = canonical_identity(role.get("family")) or canonical_identity(role.get("agent")) or canonical_identity(role.get("model"))
        if family:
            slots[slot] = family
    return slots


def load_manifest_md_files(manifest_path: Path, duel_dir: Path) -> list[Path]:
    if not manifest_path.exists():
        return []
    try:
        lines = read_text(manifest_path).splitlines()
    except OSError:
        return []
    files: list[Path] = []
    for line in lines:
        match = re.match(r"^-\s*path:\s*(planning-duel/.+\.md)\s*$", line)
        if not match:
            continue
        path = duel_dir / Path(match.group(1)).name
        if path.exists():
            files.append(path)
    return files


def determine_slot(
    path: Path,
    family: str,
    role_slots: dict[str, str],
    winner: Any,
    md_files: list[Path],
    manifest_md_files: list[Path],
) -> str | None:
    stem = path.stem
    if stem in SLOTS:
        return stem
    planner_matches = [
        slot for slot in ("planner_a", "planner_b") if role_slots.get(slot) == family
    ]
    if len(planner_matches) == 1:
        return planner_matches[0]
    if role_slots.get("arbiter") == family and "arbiter" in stem:
        return "arbiter"
    if isinstance(winner, dict):
        artifact_path = winner.get("artifact_path")
        winner_slot = winner.get("winner_slot")
        if isinstance(artifact_path, str) and Path(artifact_path).name == path.name and winner_slot in SLOTS:
            return winner_slot
    manifest_non_arbiter = [
        item for item in sorted(manifest_md_files, key=lambda item: item.name) if "arbiter" not in item.stem
    ]
    if len(manifest_non_arbiter) == 2 and path in manifest_non_arbiter:
        return "planner_a" if path == manifest_non_arbiter[0] else "planner_b"
    non_arbiter = [item for item in sorted(md_files, key=lambda item: item.name) if "arbiter" not in item.stem]
    if len(non_arbiter) == 2:
        return "planner_a" if path == non_arbiter[0] else "planner_b"
    if "arbiter" in stem:
        return "arbiter"
    return None


def rewrite_scoreboard(file_name: str, data: Any, orbit_dir: Path) -> Any:
    if file_name == "duel_plan.json" and isinstance(data, dict):
        return rewrite_duel_plan_scoreboard(data)
    if file_name == "tokens.json" and isinstance(data, dict):
        return rewrite_token_scoreboard(data)
    if file_name in {"pr.json", "task_review.json", "friction_bounty.json"} and isinstance(data, dict):
        return fold_metric_scoreboard(data)
    if file_name == "summary.json" and isinstance(data, dict):
        return rewrite_summary_scoreboard(data)
    return data


def rewrite_duel_plan_scoreboard(data: dict[str, Any]) -> dict[str, Any]:
    output = dict(data)
    runs = []
    for run in data.get("runs", []):
        if not isinstance(run, dict):
            continue
        new_run = dict(run)
        roles = {}
        for slot, assignment in (run.get("roles") or {}).items():
            if not isinstance(assignment, dict):
                continue
            family = (
                canonical_identity(assignment.get("family"))
                or canonical_identity(assignment.get("agent"))
                or canonical_identity(assignment.get("model"))
                or str(assignment.get("agent") or assignment.get("model") or "")
            )
            roles[slot] = {"family": family}
        new_run["roles"] = roles
        if "planner_a" in roles:
            new_run["planner_a_artifact_path"] = "planning-duel/planner_a.md"
        if "planner_b" in roles:
            new_run["planner_b_artifact_path"] = "planning-duel/planner_b.md"
        runs.append(new_run)
    output["runs"] = runs
    return output


def rewrite_token_scoreboard(data: dict[str, Any]) -> dict[str, Any]:
    output = dict(data)
    for section in ("activities", "agents"):
        rows = []
        for row in data.get(section, []):
            if not isinstance(row, dict):
                continue
            family = canonical_identity(row.get("agent")) or canonical_identity(row.get("model"))
            new_row = dict(row)
            if family:
                new_row["agent"] = family
                if "model" in new_row:
                    new_row["model"] = family
            rows.append(new_row)
        output[section] = rows
    return output


def fold_metric_scoreboard(data: dict[str, Any]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for metric, scores in data.items():
        if isinstance(scores, dict):
            folded: dict[str, int | float] = defaultdict(int)
            for key, value in scores.items():
                family = scoreboard_family_key(key)
                if isinstance(value, (int, float)):
                    folded[family] += value
            output[metric] = dict(sorted(folded.items()))
        else:
            output[metric] = scores
    return output


def rewrite_summary_scoreboard(data: dict[str, Any]) -> dict[str, Any]:
    output = dict(data)
    agents = data.get("agents")
    if isinstance(agents, dict):
        folded: dict[str, Any] = {}
        for key, value in agents.items():
            family = scoreboard_family_key(key)
            folded[family] = merge_score_dict(folded.get(family), value)
        output["agents"] = dict(sorted(folded.items()))
    return normalize_scoreboard_identity_values(output)


def normalize_scoreboard_identity_values(value: Any) -> Any:
    if isinstance(value, dict):
        result: dict[str, Any] = {}
        for key, child in value.items():
            if key in {"agent", "model", "role", "family"} and isinstance(child, str):
                result[key] = scoreboard_family_key(child)
            else:
                result[key] = normalize_scoreboard_identity_values(child)
        return result
    if isinstance(value, list):
        return [normalize_scoreboard_identity_values(child) for child in value]
    return value


def merge_score_dict(left: Any, right: Any) -> Any:
    if left is None:
        return right
    if isinstance(left, dict) and isinstance(right, dict):
        merged = dict(left)
        for key, value in right.items():
            merged[key] = merge_score_dict(merged.get(key), value)
        return merged
    if isinstance(left, (int, float)) and isinstance(right, (int, float)):
        return left + right
    return left


def scoreboard_family_key(label: str) -> str:
    if label.strip().lower() == "pro":
        return "gemini"
    return canonical_identity(label) or label


def scoreboard_forbidden_findings(data: Any, pointer: tuple[str, ...] = ()) -> list[str]:
    findings: list[str] = []
    if isinstance(data, dict):
        for key, value in data.items():
            key_pointer = pointer + (key,)
            if key == "by_model":
                findings.append("/".join(key_pointer))
            if key in FORBIDDEN_SCOREBOARD_TOKENS:
                findings.append("/".join(key_pointer))
            findings.extend(scoreboard_forbidden_findings(value, key_pointer))
    elif isinstance(data, list):
        for index, value in enumerate(data):
            findings.extend(scoreboard_forbidden_findings(value, pointer + (str(index),)))
    elif isinstance(data, str):
        field = pointer[-1] if pointer else ""
        identity_field = field in {"agent", "model", "role", "family", "artifact_path"}
        if identity_field and any(token in data for token in FORBIDDEN_SCOREBOARD_TOKENS):
            findings.append("/".join(pointer))
    return findings


def backup_orbit_dir(orbit_dir: Path) -> Path:
    timestamp = os.environ.get("ORBIT_FAMILY_MIGRATION_TIMESTAMP") or str(int(time.time()))
    backup_path = orbit_dir.with_name(f"{orbit_dir.name}.backup.{timestamp}")
    if backup_path.exists():
        raise SystemExit(f"backup already exists: {backup_path}")
    shutil.copytree(orbit_dir, backup_path, symlinks=True)
    return backup_path


def rel(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(Path.cwd().resolve()))
    except ValueError:
        return str(path)


def find_repo_root(start: Path) -> Path:
    current = start.resolve()
    for candidate in (current, *current.parents):
        if (candidate / "crates" / "orbit-common").exists():
            return candidate
    return start.resolve()


def check_orb_00080_precondition(repo_root: Path) -> None:
    agent_family = repo_root / "crates" / "orbit-common" / "src" / "types" / "agent_family.rs"
    agent_pair = repo_root / "crates" / "orbit-common" / "src" / "types" / "agent_pair.rs"
    has_agent_family = agent_family.exists() and "pub enum AgentFamily" in read_text(agent_family)
    resolver_present = (
        agent_pair.exists()
        and re.search(r"\bpub\s+fn\s+resolve_agent_model_pair\b", read_text(agent_pair)) is not None
    )
    if not has_agent_family or resolver_present:
        raise SystemExit(
            "ORB-00080 precondition unmet: expected AgentFamily enum to exist and "
            "resolve_agent_model_pair to be removed before running family identity migration."
        )


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Migrate Orbit persisted identity values from model strings to agent families."
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="print intended changes without writing (default)")
    mode.add_argument("--apply", action="store_true", help="create a backup and apply the migration")
    parser.add_argument("--orbit-dir", default=".orbit", help="Orbit data directory to migrate (default: .orbit)")
    parser.add_argument("--verbose", action="store_true", help="print extra discovery details")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    repo_root = find_repo_root(Path.cwd())
    check_orb_00080_precondition(repo_root)
    orbit_dir = Path(args.orbit_dir)
    if not orbit_dir.exists():
        raise SystemExit(f"orbit directory does not exist: {orbit_dir}")
    migration = Migration(orbit_dir, apply=args.apply, verbose=args.verbose)
    migration.collect()
    migration.print_report()
    migration.apply_changes()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
