<!-- orbit-managed:start -->
## Orbit Workflow Rules

- **Task before work.** File an Orbit task before non-trivial code changes. Use the `orbit-task` skill (or `orbit.task.add`). Don't invent task IDs — `orbit.task.add` allocates them.
- **Tool surface over file edits.** Use `orbit.task.*`, `orbit.docs.*`, and `orbit.learning.*` for their respective artifacts. ADRs are git-reviewed entries in feature `docs/design/**/4_decisions.md`; never edit files under `.orbit/` directly.
- **Route via the `orbit` skill.** Start sessions by reading the `orbit` skill (`<orbit-root>/skills/orbit/SKILL.md`). It is the entry point that lists every workflow skill (`orbit-task`, `orbit-workflow`, `orbit-search`, `orbit-knowledge`).
<!-- orbit-managed:end -->
