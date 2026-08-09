# Filing friction

Friction is **agent-discovered problems with Orbit itself** — unclear command behavior, missing CLI functionality, confusing schema or config, doc gaps, unclear errors, unexpected runtime behavior, confusing seed instructions, or vague activity-asset / `SKILL.md` prompts. File it instead of silently working around it.

It is **not** for task content issues, ordinary user-requested work, or generic product bugs. Those are tasks.

Search the corpus first — the point of filing is a record someone finds *before* re-diagnosing the same problem.

## Record shape

Bodies are append-only markdown under `.orbit/frictions/`; triage metadata is mutable. New records start `open` and may become `triaged` or `resolved`.

```bash
orbit tool run orbit.friction.add --input '{
  "title": "<the surface and the failure, max 120 chars>",
  "body": "<what happened, where, and why it caused friction>",
  "tags": ["<tag from table>"], "during_task": "<optional task id>",
  "model": "<agent-family>"
}'
```

**Write the `title` yourself.** It is the record's handle everywhere the corpus is scanned — `friction list`, the dashboard, and the search someone runs before filing a duplicate. A handle that doesn't name its subject is invisible to that search, which is how the same bug gets diagnosed twice. Name the surface and the failure (`orbit.task.update rejects an id orbit.task.show resolves`), not the shape of your report.

Omitting `title` is allowed and derives one from the body: the opening line, minus a leading section label, clamped to 120 characters. Derivation cannot invent a subject the opening line doesn't state, so a body opening with a section heading gets whatever that section's first sentence says. `orbit friction update --title` retitles an existing record without touching its append-only body.

## Tags

| Tag | Use for |
| --- | --- |
| `build` | make/fmt/lint friction |
| `docs` | Stale or missing instruction and design docs |
| `lifecycle` | Task lifecycle confusion or transition issues |
| `naming` | Naming drift or duplicated sources of truth |
| `policy` | fsProfile or sandboxing surprises |
| `skill-guidance` | Misleading or incorrect skill instructions |
| `tooling` | Orbit tool / CLI / MCP failures |
| `other` | Fallback |

## Lifecycle

`orbit tool run orbit.friction.update --input '{"id":"<ID>","status":"triaged"}'` (also accepts `tags` and `body`), and `orbit tool run orbit.friction.resolve --input '{"id":"<ID>"}'` to close.

When a task fixes the underlying cause, give that task `relations: [{"type":"resolves","target":"F<YYYY>-<MM>-<NNN>"}]`. It auto-resolves the friction and records `resolved_by_task` when it reaches `done`. A dangling target is audit-visible but does not block task completion.

## Rules

Never silently ignore an Orbit problem. Never implement a large design change inline — track it first. Name the concrete command, file, or workflow that broke.
