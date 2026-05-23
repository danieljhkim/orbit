---
title: Agent Observation Conventions
owner: daniel
last_updated: 2026-05-18
---

# Agent Observation Conventions

Rules for writing and maintaining files under `docs/agent-observations/`. The goal is a directory that reads as a single longitudinal log — entries comparable across windows, claims grounded in data, and a clear graduation path to lessons and ADRs.

This doc is itself the source of truth for the conventions. When a convention changes, update this doc and then update existing observation files to match — do not silently diverge.

---

## 1. Purpose

`docs/agent-observations/` is a running log of how agents (codex, claude, grok, gemini, gpt-5.x, …) actually behave when given Orbit tooling — tool-surface choice, tool reach, blind spots, failure modes — grounded in audit data, benchmark runs, or session transcripts.

This is **not** the place for:

- Distilled principles → [`../LESSONS.md`](../LESSONS.md)
- Decisions about how Orbit will change in response → ADRs in `docs/design/<feature>/4_decisions.md`
- Speculation without data

---

## 2. File Layout

Every observation has a stable ID of the form `AO-NNN` (3-digit, zero-padded, monotonic from 001, never reused, never reordered). The ID is the directory name and the canonical reference from commit messages, tasks, ADRs, and `LESSONS.md`.

Every observation is a directory — uniform shape, no flat/directory branching. Single-shot observations are just observations that never grow an `experiments/` subdirectory.

```
docs/agent-observations/
├── CONVENTIONS.md                            this file
└── AO-NNN/
    ├── observation.md                        always present — frontmatter, title, body
    ├── experiments/                          optional — present once a 2nd experiment is tracked
    │   ├── 01-<lever-slug>.md
    │   └── 02-<lever-slug>.md
    └── references/                           optional — verbatim primary-source artifacts the observation cites
        ├── task.md
        ├── plan-<family>.md
        └── ...
```

**Rules**

- Directory name is the bare `AO-NNN`. No slug, no date. The human-readable title lives inside `observation.md` (frontmatter `title:` and the H1).
- Exactly one `observation.md` per directory. It carries the canonical frontmatter (§3a) and the body sections (§4).
- The `experiments/` subdirectory is created on demand. Empty `experiments/` directories are not allowed — if you make the directory, put a file in it.
- Experiment filenames are `NN-<lever-slug>.md`. `NN` is a zero-padded chronological index (`01`, `02`, …) within that AO — never reordered after the fact.
- The `references/` subdirectory is created on demand. It holds verbatim copies of primary-source artifacts that `observation.md` or experiment files cite — task descriptions, plan artifacts, raw arbiter outputs, session logs. The point is to make the observation self-contained when the original artifacts live off-repo (e.g. in `~/.orbit/tasks/`) and could disappear or rot.
- Reference files use a small provenance header (source path, task ID, role, recorded date) then the verbatim artifact. No editorializing — analysis belongs in experiment files. Re-derivable outputs (e.g. `make ci-fast` log) are not references; only durable primary sources.
- Slug is lowercase, hyphenated, ≤ 6 words.
- One observation per AO. If you find a second pattern in the same data, allocate a new AO-NNN.
- Experiments do not get their own AO-NNN. Reference them as `AO-NNN/NN` (e.g. `AO-002/01`).
- No further nesting beyond the three top-level subdirs shown above. No `README.md`.

To scan the index, read the titles out of frontmatter:

```sh
for d in docs/agent-observations/AO-*/; do
  awk '/^title:/ {sub(/^title: */, ""); print FILENAME": "$0; exit}' "$d/observation.md"
done
```

### 2b. Growing an `experiments/` subdirectory

Start with everything in `observation.md`. Extract per-experiment content into `experiments/NN-<lever-slug>.md` files when **any** of the following hold:

- A second experiment / lever is queued or has been run against the same hypothesis.
- `observation.md` has been edited three or more times to revise hypotheses (not just to add data to an existing one).
- Two or more readers ask "what's the current hypothesis?" because the answer isn't obvious at the top.

Extraction procedure: create `experiments/`, move each experiment's narrative out of `observation.md` into its own `NN-<lever-slug>.md`, and leave only the synthesis layer in `observation.md` (frontmatter, background, TL;DR, experiment index table, cross-cutting findings, current hypothesis, open questions, reproducing). The AO-NNN never changes; no files are renamed at the directory level.

### 2c. ID allocation

To allocate the next ID:

```sh
ls docs/agent-observations | grep -E '^AO-[0-9]{3}$' | sort | tail -1
```

Take the last ID, add 1, zero-pad to 3 digits. Never reuse an ID — if an observation is retracted, leave the directory in place with a `status: retracted` note rather than freeing the slot.

---

## 3. Required Frontmatter

### 3a. `observation.md` frontmatter

Every `observation.md` starts with YAML frontmatter:

```yaml
---
id: AO-NNN
title: <one-line headline>
owner: <agent family — codex, claude, grok, gemini>
last_updated: YYYY-MM-DD
recorded: YYYY-MM-DD
tags: [tag1, tag2, ...]
---
```

- `id` matches the parent directory name. Required, never changes.
- `title` mirrors the H1 verbatim. This is the human-readable name that the bare `AO-NNN` directory doesn't carry.
- `owner` is the agent family, not a full model string.
- `last_updated` is the calendar date the file last had a meaningful edit. Distinct from `recorded`, which is fixed at creation.
- `recorded` is the date the observation was made. Never changes after the file is created.
- `tags` use lowercase-hyphenated form. Reuse existing tags before inventing new ones — grep the directory first.

The data window and source belong in the body (typically a metadata block under the H1, before the TL;DR) — not in frontmatter. They're prose context, not machine-readable indexing.

### 3b. Experiment-file frontmatter

Files under `experiments/` use a smaller, lever-focused frontmatter:

```yaml
---
parent: ../observation.md
recorded: YYYY-MM-DD
lever: <short noun phrase describing what was changed>
status: pending | running | refuted | confirmed | inconclusive
artifact: <task id, commit sha, or jrun id>
---
```

- `parent` is always the literal string `../observation.md` — readers cd into the experiment and need a back-link.
- `lever` names the independent variable (e.g. "GEMINI.md presence", "PLANNING_DUEL_INSTRUCTION rubric"). One per file.
- `status` transitions monotonically: `pending → running → {refuted | confirmed | inconclusive}`. Once terminal, do not re-edit; write a follow-up experiment.
- `artifact` points to the receipt — the task ID that produced the data, the commit that landed the change, or the duel run ID.

Experiment files do not need `owner`, `title`, `last_updated`, or `tags` — those are inherited from the parent.

---

## 4. Required Sections

### 4a. `observation.md`

| Section | Required | Purpose |
|---|---|---|
| `# <title>` | yes | Matches frontmatter `title`. |
| `## Background` | recommended | Why this observation exists, baseline state at observation start. |
| `## TL;DR` | yes | Two or three sentences. The headline finding, stated as a claim. Update on every meaningful edit. |
| Data tables | yes | At least one. Raw counts plus a derived ratio. Markdown tables only — no images, no charts. When `experiments/` exists, per-experiment data tables live in the experiment files; observation.md carries cross-cutting tables only. |
| `## Experiments` | yes when `experiments/` exists | Index table linking to each `experiments/NN-*.md` with one-line status. No raw data here. |
| `## Current hypothesis` (active threads) or `## Findings` (closed threads) | yes when `experiments/` exists | For active threads: the hypothesis the next experiment will test — when refuted, archive the prior hypothesis in the relevant experiment file and replace this section. For closed threads (`**status:** closed` under the H1): the synthesized cross-experiment claims, with each claim grounded in a specific experiment file. |
| `## Caveats` | yes | What this data does not prove. Sample size, attribution risks, selection bias. |
| `## Open questions` | recommended | Things that would sharpen or refute the finding if we had more data. |
| `## Reproducing this` | recommended | The exact command(s) and jq pipeline used. When `experiments/` exists, this section carries commands that span experiments; per-experiment commands live in the experiment file. |

### 4b. `experiments/NN-*.md` files

| Section | Required | Purpose |
|---|---|---|
| `# Experiment NN — <lever>` | yes | Mirrors `lever` from frontmatter. |
| `## Setup` | yes | What was changed, when, and where (commit / file / config). |
| `## Result` | yes | Outcome. Data tables here, not in `observation.md`. State the verdict against the hypothesis explicitly. |
| `## Reproducing` | recommended | Commands specific to this experiment. |

Other sections (mechanism, hypotheses, knock-on effects) are encouraged where they earn their place. Skip them if there's nothing to say.

---

## 5. Claim Standard

- **Ratios are load-bearing; absolute counts are suggestive.** Cross-model comparisons must normalize. If you cite a raw number, state what's in the denominator.
- **Same-window comparisons only.** Don't compare a 3-day codex slice to a 30-day claude slice.
- **Distinguish behavior from wiring.** Before claiming a model prefers X, verify the alternative was actually available and equally easy to reach.
- **Attribute carefully.** The `role` field in audit events comes from the caller's self-identification. A row tagged `claude` may be a human invocation in a claude session.

---

## 6. Graduation

If a behavior holds across multiple windows and starts shaping product decisions:

- Lift the **principle** into [`../LESSONS.md`](../LESSONS.md). Cite the observation by ID (e.g. "from `AO-002`").
- Capture the **response** as an ADR in the relevant `docs/design/<feature>/4_decisions.md`. Cite the observation by ID in the ADR body.
- Leave the raw observation file in place. It's the receipt.

Do not edit the original observation to backfill what we later did about it. Write a follow-up observation (new AO-NNN) that cites the original by ID.

---

## 7. What Not to Do

- Don't edit an observation's `recorded` date or `id` after the fact. If the data changes, write a new file (new AO-NNN).
- Don't reuse a retired AO-NNN. Skip the slot — never recycle.
- Don't author observations on data you didn't pull yourself or can't reproduce.
- Don't bury the finding under prose. The TL;DR and the first table should carry the claim; everything else is supporting evidence.
- Don't speculate about model internals beyond what the data warrants. "OpenAI models picked CLI 93% of the time" is a finding; "OpenAI models were trained to prefer shell" is a hypothesis — label it as one.
