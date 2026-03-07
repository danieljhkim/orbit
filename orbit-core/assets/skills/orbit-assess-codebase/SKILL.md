---
name: orbit-assess-codebase
description: Use for a comprehensive evaluation of the codebase to identify strengths, weaknesses, risks, and improvement opportunities. This skill is typically requested periodically by scheduled orbit-job, orbit-task, or human. Use this skill only when explicitly requested to "orbit-assess-codebase". 
---

# Orbit Assess Codebase

## Purpose

Perform a holistic, senior-level evaluation of the project, focusing on:

- System architecture and modularity
- Code quality and maintainability
- Security posture
- Performance and efficiency
- Developer experience (DX)
- User experience (UX), if applicable
- Testing strategy and coverage
- Operational readiness

Deliver:

- A structured assessment
- Clear strengths and weaknesses
- Risk analysis
- Prioritized improvement roadmap
- Thoughtful feature expansion ideas

All findings of issues must be emitted and audited in a structured form suitable for issue tracking. New issues must be tracked by utilizing `orbit-track-issues` skill.

---

## Assessment Workflow

1. Understand the project’s purpose and target users.
2. Review repository structure, layering boundaries, and dependency flow.
3. Evaluate testing strategy, determinism, and failure-path coverage.
4. Assess performance characteristics, scaling risks, and operational readiness.
5. Review security posture and threat surface.
6. Identify UX/DX friction, architectural smells, and technical debt.
7. Propose a prioritized improvement roadmap and feature opportunities.
8. Turn each issue finding into a tracked issue by invoking `orbit-track-issues` (do not create issue files directly).
9. Write the assessment to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD-<title>.md`.

Assessment reports are global outputs. If you create any artifact that belongs to exactly one Orbit task during follow-up work, store that artifact under the task bundle's `artifacts/` directory instead of `agents/reports/`.

---

## Evaluation Dimensions

### 1. Architecture & Design
- Separation of concerns
- Modularity and layering
- Extensibility
- Dependency management
- Coupling and cohesion

### 2. Code Quality
- Readability
- Naming clarity
- Error handling discipline
- Logging and observability
- Consistency and standards adherence

### 3. Security
- Input validation
- Authentication / authorization (if applicable)
- Secret management
- Data exposure risks
- Attack surface assessment

### 4. Performance & Efficiency
- Algorithmic complexity
- I/O behavior
- Memory usage patterns
- Concurrency safety
- Scaling bottlenecks

### 5. Testing & Reliability
- Unit test coverage quality (not just percentage)
- Integration testing strategy
- Determinism of tests
- Failure handling paths
- CI enforcement rigor

### 6. Developer Experience (DX)
- Build and setup clarity
- Local development reproducibility
- Tooling and automation
- Documentation completeness
- Onboarding friction

### 7. User Experience (UX)
- Interface clarity (CLI, API, UI)
- Error messages quality
- Feedback loops
- Discoverability

---

## Output Structure

```markdown
# Codebase Assessment — <Project Name>

## Executive Summary
Brief high-level evaluation and overall health classification.

Overall Health: Excellent / Strong / Moderate / At Risk

---

## Strengths
- <key strengths>

---

## Key Risks
- Risk:
  - Severity: Low / Medium / High
  - Impact:
  - Recommendation:
  - Production Blocking: Yes / No

---

## Dimension-by-Dimension Analysis

### Architecture & Design
<analysis>

### Code Quality
<analysis>

### Security
<analysis>

### Performance & Efficiency
<analysis>

### Testing & Reliability
<analysis>

### Developer Experience
<analysis>

### User Experience
<analysis>

---

## Prioritized Improvement Roadmap

### Immediate (High Impact / Low Effort)
- <action>

### Near-Term
- <action>

### Long-Term
- <action>

---

## Strategic Feature Opportunities
- <new feature idea>
- <expansion direction>

---

## Final Assessment
Clear summary of project maturity level and trajectory.

Production Sign-Off: YES / NO

If NO:
- Blocking Reasons:
  - <reason>
```

---

## Assessment Rules

- Be objective and evidence-driven.
- Distinguish observation from recommendation.
- Avoid vague statements; be concrete.
- Identify trade-offs explicitly.
- Do not implement or rewrite code during assessment.
- If context is insufficient, ask focused clarifying questions.
- Use ISO date format (YYYY-MM-DD) in filenames and dates.
- Do not overwrite existing assessments unless explicitly instructed.
- Treat each assessment as a point-in-time snapshot.
- If profile.yaml enables auto issue creation, ensure all findings at or above threshold include clear Title, Severity, Impact, and Recommended Action fields.
- Production Sign-Off must be NO if any High severity Production Blocking risk exists.
- When creating issues from findings, the agent must delegate issue creation to the `orbit-track-issues` skill rather than writing issue files directly.
- For delegated issue task creation, attribution should use identity details when available; otherwise use model name fallback for `assigned_to` and `created_by`, and set `identity_id` only when model-alias identity exists.
