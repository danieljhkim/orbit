---
name: orbit-assess-codebase
description: Use for a comprehensive evaluation of the codebase to identify strengths, weaknesses, risks, and improvement opportunities. This skill is typically requested periodically by scheduled orbit-job, orbit-task, or human. Use this skill only when explicitly requested to "orbit-assess-codebase". 
---

# Assess Codebase

## Purpose

This skill performs a holistic evaluation of the project as if conducted by a senior or principal-level engineer.

It analyzes:

- System architecture and modularity
- Code quality and maintainability
- Security posture
- Performance and efficiency
- Developer experience (DX)
- User experience (UX), if applicable
- Testing strategy and coverage
- Operational readiness

The goal is to provide:

- A structured assessment
- Clear strengths and weaknesses
- Risk analysis
- Prioritized improvement roadmap
- Thoughtful feature expansion ideas

This skill does not implement changes. It evaluates and recommends.

All findings of issues must be emitted and audited in a structured form suitable for issue tracking. New issues must be tracked by utilizing `orbit-track-issues` skill.

---

## Assessment Workflow

1. Understand the project’s purpose and target users.
2. Review repository structure and architectural boundaries.
3. Evaluate code organization and dependency flow.
4. Analyze testing strategy and verification rigor.
5. Evaluate performance characteristics and scaling risks.
6. Review security considerations and threat surface.
7. Evaluate UX and DX friction points.
8. Identify architectural smells or technical debt.
9. Propose improvements and new feature opportunities.
10. Provide a prioritized roadmap.
11. Prepare structured findings for auto issue creation.
12. Invoke the `orbit-track-issues` skill to create or update corresponding pending issues.
13. output the result as a markdown file in the `~/.orbit/agents/<repo_name>/reports/YYYY-MM-DD-<title>.md` directory.

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
- Do not rewrite code during assessment.
- If context is insufficient, ask focused clarifying questions.
- Use ISO date format (YYYY-MM-DD).
- Do not overwrite existing assessments unless explicitly instructed.
- Each assessment should represent a point-in-time evaluation.
- If profile.yaml enables auto issue creation, ensure all findings at or above threshold include clear Title, Severity, Impact, and Recommended Action fields.
- Production Sign-Off must be NO if any High severity Production Blocking risk exists.
- When creating issues from findings, the agent must delegate issue creation to the `orbit-track-issues` skill rather than writing issue files directly.
