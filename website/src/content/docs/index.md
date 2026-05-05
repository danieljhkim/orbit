---
title: What Orbit Is
description: "Orbit is a self-hosted runtime for fleets of coding agents — scoped, replayable, and bounded by policies you wrote."
tableOfContents: false
---

<section class="orbit-hero">
  <div class="orbit-hero-copy">
    <div class="orbit-hero-eyebrow">v0.3 · early access</div>
    <h1 class="orbit-hero-headline">An agent runtime for engineering work.</h1>
    <p class="orbit-hero-lede">Define a task, attach activities, dispatch agents in parallel. Every run is replayable and bounded by policies you wrote — with OS-level <code>sandbox-exec</code> isolation on macOS.</p>
    <div class="orbit-hero-install">
      <span class="orbit-hero-install-prompt">$</span>
      <code>curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh</code>
      <button class="orbit-hero-install-copy" type="button" data-copy="curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh">Copy</button>
    </div>
    <div class="orbit-hero-actions">
      <a class="orbit-button primary" href="./getting-started/install/">Install Orbit →</a>
      <a class="orbit-button" href="https://github.com/danieljhkim/orbit">GitHub</a>
    </div>
  </div>
  <div class="orbit-hero-diagram" aria-hidden="true">
    <span class="orbit-dot"></span>
    <span class="orbit-dot orbit-dot-outer"></span>
    <span class="orbit-core"></span>
  </div>
</section>

<div class="orbit-section-title">Start here</div>

<div class="orbit-card-grid">
  <a class="orbit-card" data-tag="01" href="./getting-started/install/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg></div>
    <h3>Install Orbit</h3>
    <p>One binary, zero config. Bring your own model provider.</p>
  </a>
  <a class="orbit-card" data-tag="02" href="./getting-started/first-task/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg></div>
    <h3>Your first task</h3>
    <p>Define scope, attach activities, watch agents work in parallel.</p>
  </a>
  <a class="orbit-card" data-tag="03" href="./concepts/activities-jobs/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.91a1 1 0 0 0 0-1.83z"/><path d="M22 17.65l-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="M22 12.65l-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/></svg></div>
    <h3>Activities &amp; jobs</h3>
    <p>The atomic unit of work. Composable, replayable, scoped by filesystem policy.</p>
  </a>
  <a class="orbit-card" data-tag="04" href="./concepts/knowledge-graph/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="5" cy="6" r="3"/><path d="M5 9v6"/><circle cx="5" cy="18" r="3"/><path d="M12 3v18"/><circle cx="19" cy="6" r="3"/></svg></div>
    <h3>Knowledge graph</h3>
    <p>Tasks accrete context. Every run sharpens the next.</p>
  </a>
  <a class="orbit-card" data-tag="05" href="./reference/cli/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg></div>
    <h3>CLI reference</h3>
    <p>Every command, flag, and exit code. Cross-linked.</p>
  </a>
  <a class="orbit-card" data-tag="06" href="./reference/scoping/">
    <div class="orbit-card-icon" aria-hidden="true"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg></div>
    <h3>Policies</h3>
    <p>Declarative scoping rules for what an agent can touch.</p>
  </a>
</div>

<div class="orbit-section-title">Why Orbit</div>

<div class="orbit-design-notes">
  <div class="orbit-design-note">
    <div class="orbit-design-note-key">Self-hosted</div>
    <div class="orbit-design-note-value">Source never leaves your infrastructure. Bring your own model provider.</div>
  </div>
  <div class="orbit-design-note">
    <div class="orbit-design-note-key">Auditable</div>
    <div class="orbit-design-note-value">Every agent action is captured. Replay any run, any time.</div>
  </div>
  <div class="orbit-design-note">
    <div class="orbit-design-note-key">Scope-first</div>
    <div class="orbit-design-note-value">Filesystem policies bound exactly what an agent can touch.</div>
  </div>
  <div class="orbit-design-note">
    <div class="orbit-design-note-key">Concurrent</div>
    <div class="orbit-design-note-value">Worktree isolation lets agents work in parallel without colliding.</div>
  </div>
</div>

<script is:inline>
  document.addEventListener("click", (e) => {
    const btn = e.target.closest(".orbit-hero-install-copy");
    if (!btn) return;
    const text = btn.dataset.copy || "";
    if (!text || !navigator.clipboard) return;
    navigator.clipboard.writeText(text).then(() => {
      const prev = btn.textContent;
      btn.textContent = "Copied";
      setTimeout(() => { btn.textContent = prev; }, 1400);
    });
  });
</script>
