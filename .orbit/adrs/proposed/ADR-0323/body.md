## Context

A friction record's handle was not a field. `title` existed only on the wire: the read projection derived it from the body's first non-empty line, stripped leading `#` characters, and returned the whole line. No write surface accepted a title, so no author could set one, and nothing in the tool schemas or the authoring skill said the first line was load-bearing.

A survey of a mature corpus (41 records, two agent families) found the derivation tracked authoring style rather than content. Records written as headingless prose derived a descriptive opening sentence by accident of style. Records written as structured reports — a leading section heading followed by sibling headings — derived that section label as their title, identifying nothing. A record written as one long lead paragraph derived the entire 700-character paragraph. Both failure modes are the same missing field seen from opposite ends.

The cost is measurable rather than cosmetic. Two records six days apart documented the same underlying bug, each rediagnosed from scratch. Both carried the same generic section label as their handle, so a search for prior art before filing surfaced nothing recognisable. The corpus is meant to be small and high-signal; a record whose handle does not name its subject is invisible to the person deciding whether a problem is already known.

## Decision

**1. `title` is a stored, author-settable field.** `FrictionRecord` and its frontmatter gain `title: Option<String>`. `orbit.friction.add` accepts it; `orbit.friction.update` can set or clear it, so a record can be retitled without touching its append-only body. The dashboard's create and patch bodies accept it too, giving human triage the same power as the tool surface.

**2. Derivation runs at write time and stays as the read-side fallback.** An add that supplies no title derives one and persists it, so the file itself states the handle and the next reader can see and correct it. Records written before the field existed carry no `title` and derive one on read, so the existing corpus stays readable with no migration and no body rewrites.

**3. Derivation reads structure, not vocabulary.** Two rules, both language- and author-independent:

- A leading ATX heading is the record's own title only when no later heading at its level or shallower follows it. A heading with siblings labels the first *section* of a structured report, so the subject is the prose it introduces and derivation skips the label.
- A leading `**bold**` run that opens a prose line is an inline lead-in labelling the sentence beside it, so that sentence is the subject. A line that is nothing but the bold run is itself the subject.

The result is clamped to `FRICTION_TITLE_MAX_CHARS` (120) at a word boundary. An author-supplied title is validated against the same bound and collapsed to one line; past it the write is refused rather than silently truncated, because the author can fix what a truncation would guess at.

**4. There is no `summary` field, deliberately.** A survey of the code found none: no store field, no schema parameter, no projection. What consumers call a summary is either the record's `title` or a client-side truncation of `body`. The record keeps exactly one short handle plus the full report, so `title` and `summary` are unified by construction rather than by accident.

**Rejected: a list of generic section headings to skip.** It is the shape the symptom suggests and the wrong mechanism. It encodes one language and one house style, it needs an edit every time an author invents a new label, it cannot help the overlong-paragraph failure at all, and it treats a symptom of the missing field rather than the missing field. The corpus itself refutes it as necessary: heading count alone separated every well-titled record from every badly-titled one, with no word ever consulted.

**Rejected: rejecting an add whose derived title looks non-identifying.** A write gate would force every structured-report author to pass a title explicitly, which is the correct nudge but a hard break for existing callers (the bridge MCP server, the dashboard, machine-filed triage frictions), and "looks non-identifying" is the same brittle judgement the heading list makes. The skill text asks for the title; the structural rules make the fallback usable when it is not supplied.

## Consequences

- Every friction lands with a handle that names its subject: an author's own title, or the first prose statement of the body, never a bare section label and never an unreadable paragraph.
- The existing corpus self-heals on read for the section-label and overlong cases. A record whose body genuinely never states its subject still needs a human title; `update --title` is the supported way to give it one.
- Title validation lives in one place (`orbit_common::friction::title`) and both tool-host implementations — the checkout-backed host and the checkoutless hub coordination executor — call it, so the two write paths cannot drift on what a legal title is.
- Cost: the MCP tool schema for `orbit.friction.add` and `.update` gains a parameter. The addition is additive and optional — every existing call keeps working unchanged — but it is still a tool-surface change, and the snapshot guard treats schema drift as release-visible.
- Cost: two structural rules are more code than a first-line read, and they can still be wrong. A body whose first section genuinely holds the subject in its second sentence derives a weaker title than a careful author would write. The stored field is the escape hatch, which is why it is the primary mechanism and derivation only the fallback.
- Cost: the deployed `orbit` binary and the Bridge MCP server must be rebuilt before `--title` is reachable from a live surface; until then existing records cannot be retitled through the tools.