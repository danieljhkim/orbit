---
name: orbit-design
description: Use this when scaffolding, listing, inspecting, or decay-checking design-doc feature folders under `docs/design/<feature>/` via `orbit.design.*`. Covers the four-numbered-doc layout, the `Last updated:` decay anchor, the ADR earning rule, and why never to hand-create design folders or edit `Last updated:` to fake freshness.
---

# Orbit Design

## Purpose

Manage the design-doc system through the registered tool surface. Design docs are Orbit's convention for capturing architectural intent: every load-bearing feature gets a folder under `docs/design/<feature>/` with four numbered docs (overview, design, vision, decisions) plus `specs/` and `references/`. The convention rulebook lives in [docs/design/CONVENTIONS.md](../../../docs/design/CONVENTIONS.md); the meta design folder for the design-docs system itself is [docs/design/design-docs/](../../../docs/design/design-docs/).

Use this skill when: starting a new feature folder, auditing an existing folder before review, running the decay check before declaring a doc "current," or pulling the on-disk state of any folder programmatically. Do **not** use it for prose authoring inside a doc — use file edits for that, then re-run `orbit design check` to confirm the `Last updated:` line is correct.

## Tool Invocation

Both surfaces accept the same JSON. Use the CLI form when shell access is available; use the MCP names when the Orbit plugin exposes them.

| Tool | MCP | CLI |
|------|-----|-----|
| `orbit.design.init` | `orbit_design_init({...})` | `orbit tool run orbit.design.init --input '{"feature":"my-feature","owner":"claude"}'` |
| `orbit.design.list` | `orbit_design_list({...})` | `orbit tool run orbit.design.list --input '{}'` |
| `orbit.design.show` | `orbit_design_show({...})` | `orbit tool run orbit.design.show --input '{"feature":"my-feature"}'` |
| `orbit.design.check` | `orbit_design_check({...})` | `orbit design check [--warn-only] [--include-missing]` |

Mapping rule: `orbit.design.<verb>` ↔ `orbit_design_<verb>`. Always include `model` in JSON inputs when the tool accepts it. The `check` tool is the only one with a top-level `orbit design <subcommand>` CLI shortcut today; the other three are reachable through `orbit tool run orbit.design.<verb>`.

`make check-design-docs` shells out to `orbit design check` and runs as part of `make ci`. The legacy `scripts/check_design_doc_decay.py` is a thin compatibility wrapper around the same binary.

## Workflow

1. **Inspect before scaffolding.** Before adding a new folder, list what exists:
   - `orbit tool run orbit.design.list --input '{}'` returns one record per non-`_`-prefixed folder under `docs/design/` with per-doc `decay_status`, `owner`, `last_updated`, and absolute paths.
   - `orbit tool run orbit.design.show --input '{"feature":"<feature>"}'` returns the same shape for one feature, with a typed `design_feature_not_found` error when absent.
   If a folder for a closely-related concern already exists, prefer extending it (specs/ entries, mechanism sections in `2_design.md`) over creating a new sibling — the convention's [CONVENTIONS.md §10](../../../docs/design/CONVENTIONS.md) anti-pattern table rejects "tiny features that should have lived in an existing folder."

2. **Scaffold via `init`.** `orbit tool run orbit.design.init --input '{"feature":"<lowercase-hyphenated>","owner":"<agent-id>"}'` creates the folder atomically: four numbered docs (`1_overview.md`, `2_design.md`, `3_vision.md`, `4_decisions.md`) with required frontmatter pre-populated (today's date, the supplied `owner`), plus empty `specs/` and `references/` subfolders. The tool refuses to clobber an existing folder — delete or move first if you really mean to re-init. Feature names must be lowercase, hyphenated, and path-safe; the validator rejects spaces and uppercase.

3. **Author the four docs.** Edit the scaffolded files directly. Required sections per doc role are listed in [CONVENTIONS.md §3](../../../docs/design/CONVENTIONS.md). Common gotcha: `init` writes the `## Task References` section into `4_decisions.md` *above* where ADRs go — move it to the bottom (after the last ADR) before authoring entries, since ADRs are append-only and Task References belongs at end-of-file per existing folder convention.

4. **Run `check` before review.** `orbit design check` exits 1 when any doc's `Last updated:` is older than the most recent commit on a referenced source file. The output names the doc, the declared date, and each newer reference with its commit date. Fix by either (a) updating the prose and bumping `Last updated:` to today, or (b) bumping `Last updated:` alone if you have read the doc end-to-end and confirmed it still describes the system.

5. **Strict link-check pass.** Run `orbit design check --include-missing` to also fail on markdown links to files that no longer exist. Default mode does not fail on missing references because broken-link cleanup is sometimes a separate PR — opt into strict mode at review time.

6. **Re-show to confirm.** After edits, `orbit tool run orbit.design.show --input '{"feature":"<feature>"}'` returns each doc's `decay_status: "fresh"` once `Last updated:` is current; `"stale"` otherwise. Use this as the post-edit sanity check before declaring the folder ready.

## Operating Rules

- **Never scaffold a feature folder by hand.** Drift between sibling folders is the failure mode the convention exists to prevent. Use `orbit.design.init` so the four-doc layout, frontmatter, and section skeletons are correct from byte one.
- **Never bump `Last updated:` without reading the doc end-to-end.** That field is an explicit author assertion (see [docs/design/design-docs/4_decisions.md ADR-002](../../../docs/design/design-docs/4_decisions.md)). Cosmetic-only PRs do not bump the date; the field is the freshness signal, not file mtime.
- **Folder names are lowercase, hyphenated, singular.** The tool validates this; do not work around with underscores or PascalCase.
- **`init` does not clobber.** If you ran it with the wrong feature name, delete the folder and re-run; do not patch in place.
- **Required sections are non-negotiable.** Every numbered doc needs the sections listed in [CONVENTIONS.md §3](../../../docs/design/CONVENTIONS.md). Reviewers reject folders missing them; future lint may enforce mechanically.
- **ADRs use the 3-of-3 earning rule.** A decision belongs in `4_decisions.md` only when it has a real alternative, a forward constraint, and a non-trivial cost. Use the `orbit-adr` skill for the ADR artifact surface; the `orbit.design.*` tools manage *folders*, not individual ADRs.
- **`check` failures block `make ci`.** A stale-doc finding is treated like a clippy warning under `-D warnings`; do not skip it with `--warn-only` in CI configurations.
- **`_archive/` is the retirement path.** When a feature is retired, move the folder under `docs/design/_archive/<feature>/` and annotate the first line of `1_overview.md`. Tools skip `_`-prefixed folders automatically.

## Minimal Commands

Scaffold a new feature folder:

```bash
orbit tool run orbit.design.init --input '{"feature":"my-feature","owner":"claude-opus-4-7"}' --pretty
```

List every existing feature folder with per-doc decay status:

```bash
orbit tool run orbit.design.list --input '{}' --pretty
```

Show one feature's metadata:

```bash
orbit tool run orbit.design.show --input '{"feature":"knowledge-graph"}' --pretty
```

Run the decay check (default mode — exit 1 on stale):

```bash
orbit design check
```

Strict pass before review (also fail on broken links):

```bash
orbit design check --include-missing
```

Diagnostics-only mode (print findings, exit 0):

```bash
orbit design check --warn-only
```

## Common Mistakes

| Mistake | Why it fails | Correct form |
|---------|--------------|--------------|
| Copying a sibling folder by hand to start a new feature | Drift between sibling folders compounds; missed required sections; wrong frontmatter dates | `orbit tool run orbit.design.init --input '{"feature":"...","owner":"..."}'` |
| Bumping `Last updated:` to silence the decay check without re-reading the doc | The field is an explicit author assertion; lying breaks the freshness signal | Read the doc end-to-end, then bump |
| Adding a fifth top-level file (`README.md`, `roadmap.md`) under a feature folder | Anti-pattern table in [CONVENTIONS.md §10](../../../docs/design/CONVENTIONS.md) rejects it; readers expect exactly four numbered docs | Fold the content into the appropriate numbered doc, or into a `specs/` entry |
| Running `init` then re-running it after editing the wrong feature name | `init` refuses to clobber existing folders | Delete or move the wrong folder first, then re-run |
| Using `orbit design check` to "lint" content | The tool only checks decay (date vs. referenced-code commit); section completeness is review-enforced today | Cross-review against [CONVENTIONS.md §3](../../../docs/design/CONVENTIONS.md) |

## Exit Criteria

The feature folder exists with the four numbered docs and two subfolders, each numbered doc has valid frontmatter (`Status`, `Owner`, `Last updated:`), `orbit design check` exits 0 against the workspace, and `orbit tool run orbit.design.show --input '{"feature":"<feature>"}'` reports every doc as `decay_status: "fresh"`.
