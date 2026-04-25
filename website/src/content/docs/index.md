---
title: What Orbit Is
description: "A two-minute orientation to Orbit and the documentation paths that matter first."
---

<section class="orbit-hero">
  <div class="orbit-hero-copy">
    <p>Orbit is a self-hosted runtime for running fleets of coding agents against your team's real codebase. It combines a code-aware graph, task locks, isolated worktrees, and structured audit events so parallel agent work remains inspectable.</p>
    <pre class="orbit-hero-install"><code>curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh</code></pre>
    <div class="orbit-hero-actions">
      <a class="orbit-button primary" href="./getting-started/install/">Install Orbit</a>
      <a class="orbit-button" href="./reference/cli/">CLI Reference</a>
      <a class="orbit-button" href="./architecture/">Architecture</a>
    </div>
  </div>
  <div class="orbit-hero-diagram" aria-hidden="true">
    <span class="orbit-dot"></span>
    <span class="orbit-core"></span>
  </div>
</section>

## Start Here

Orbit is for staff engineers and platform leads at small teams who want agent automation to run against production repositories without sending source through a hosted agent platform. It is not a generic workflow engine and it is not a personal coding assistant wrapper.

Use these paths first:

<div class="orbit-card-grid">
  <a class="orbit-card" href="./getting-started/first-task/">
    <h3>Run a first task</h3>
    <p>Create a durable task, inspect it, and ship it through the default execution path.</p>
  </a>
  <a class="orbit-card" href="./concepts/activities-jobs/">
    <h3>Understand execution</h3>
    <p>Learn how activities and jobs model agent loops, shell steps, retries, and fan-out.</p>
  </a>
  <a class="orbit-card" href="./reference/activity-job-yaml/">
    <h3>Write YAML</h3>
    <p>Use the reference shapes for activity and job assets.</p>
  </a>
  <a class="orbit-card" href="./architecture/design/">
    <h3>Read the design mirror</h3>
    <p>Browse the source architecture docs from `docs/design/` in site form.</p>
  </a>
</div>

## Product Shape

Orbit centers on four concepts:

- **Task:** a durable unit of work with state, acceptance criteria, review, and audit history.
- **Knowledge graph:** parsed repository structure used by agents and schedulers instead of raw text search.
- **Worktree:** an isolated git checkout for each agent session.
- **Locks:** explicit file or code-region claims that keep concurrent agent sessions from colliding.

Lower-level resources such as activities, jobs, policies, tools, and executors are intentionally visible. You inspect and change them because Orbit's trust model depends on being able to answer what happened, why it happened, and which agent did it.

## Boundaries

Orbit deliberately avoids lead capture, hosted demos, interactive playgrounds, and marketing pages. This site follows the same line: search, reference, and task-oriented documentation first.
