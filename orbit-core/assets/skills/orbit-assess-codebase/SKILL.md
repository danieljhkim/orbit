---
name: orbit-assess-codebase
description: Use for a comprehensive evaluation of the codebase to identify strengths, weaknesses, risks, and improvement opportunities. This skill is typically requested periodically by scheduled orbit-activity, orbit-task, or human. Use this skill only when explicitly requested to "orbit-assess-codebase".
---

# Orbit Assess Codebase

## Purpose

Perform a structured, point-in-time evaluation of the codebase across architecture, quality, security, performance, testing, operations, DX, and UX. Route every issue finding into `orbit-track-issues`.

## Workflow

1. Understand the project purpose and target users.
2. Review repository structure, layering, and dependency flow.
3. Evaluate code quality, observability, and error handling.
4. Assess security, performance, and operational risks.
5. Review testing strategy, determinism, and failure-path coverage.
6. Identify UX/DX friction, technical debt, and architectural weaknesses.
7. Produce a prioritized improvement roadmap and feature opportunities.
8. Route each issue finding through `orbit-track-issues`.
9. Write the report to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD/assessment_<title>.md`.
10. Commit the assessment along with related job-run artifacts under `.orbit/jobs/jobs/runs/<job_run_id>/`

## Evaluation Dimensions

- Architecture & design: layering, coupling, extensibility, dependency flow
- Code quality: readability, naming, consistency, observability, error handling
- Security: validation, secrets, exposure risks, attack surface
- Performance & efficiency: complexity, I/O, memory, concurrency, scaling
- Testing & reliability: determinism, coverage quality, failure paths, CI rigor
- Developer experience: setup, tooling, docs, onboarding
- User experience: CLI/API/UI clarity, feedback, error quality

## Output Structure

```markdown
# Codebase Assessment - <Project Name>

## Executive Summary
Overall Health: Excellent / Strong / Moderate / At Risk

## Strengths
- <strength>

## Key Risks
- <risk> | Severity: Low / Medium / High | Blocking: Yes / No | Impact: <impact> | Recommendation: <action>

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

## Prioritized Improvement Roadmap
### Immediate
- <action>
### Near-Term
- <action>
### Long-Term
- <action>

## Strategic Feature Opportunities
- <idea>

## Final Assessment
Production Sign-Off: YES / NO
If NO: <blocking reasons>
```

## Rules

- Be evidence-driven and distinguish observation from recommendation.
- Be concrete; name trade-offs explicitly.
- Do not implement code during the assessment.
- Use ISO dates in filenames and report content.
- Do not overwrite prior assessments unless explicitly instructed.
- Production Sign-Off must be NO for any High severity production-blocking risk.
- Delegate issue creation to `orbit-track-issues`; do not create issue files directly.
