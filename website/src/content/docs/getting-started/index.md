---
title: Getting Started
description: "Install Orbit, initialize state, run a first task, and inspect available activities."
sidebar:
  order: 1
---

## Path

Use this section when you are setting up Orbit for the first time.

<div class="orbit-card-grid">
  <a class="orbit-card" href="./install/">
    <h3>Install Orbit</h3>
    <p>Install through npm, a trusted installer checkout, or a source build.</p>
  </a>
  <a class="orbit-card" href="./first-task/">
    <h3>First Task</h3>
    <p>Create a task, inspect it, and ship it.</p>
  </a>
  <a class="orbit-card" href="./workflows/">
    <h3>Default Workflows</h3>
    <p>Run the built-in ship workflow.</p>
  </a>
</div>

## Prerequisites

You need an authenticated supported provider CLI: agent activities dispatch through it.
PR mode also requires the GitHub CLI to be authenticated in the environment
where Orbit runs.

Orbit itself can be installed without Rust. You only need a Rust toolchain if you build from source or contribute to the Rust workspace.
