---
name: orbit-operations-management
description: ONLY use this skill when you have been explictly told to use "orbit-operations-management" skill verbatim. As a CEO, use this skill to perform a periodic operational review of Orbit runtime activity. 
---

# Purpose

This skill performs a lightweight operational audit of Orbit activity. The agent acts as the "CEO of orbit-workflow operations" and audits recent job executions

Responsibilities:

- Inspect recent job runs
- Detect failures or abnormal states
- Investigate root causes
- Create Orbit tasks for remediation

The goal is to maintain a healthy and reliable Orbit runtime environment.

---

# Procedure

Follow the steps below strictly.

## 0. Retrieve Successful Job Runs

List all success executions:

```
orbit job-run list --status success
```

Review each job-runs, and archive them if looks good, using `orbit job-run archive <job_run_id>`

## 1. Retrieve Failed Job Runs

List all failed executions:

```
orbit job-run list --status failed
```

If no failures are present, report that operations are healthy and stop.

---

## 2. Inspect Each Failed Run

For every returned `job_run_id`:

```
orbit job-run show <job_run_id>
```

Review:

- job_id
- command executed
- error message
- exit code
- timestamps

Determine the likely cause of failure.

---

## 3. Create Remediation Task

For each failure, create a task of type `issue`.

Example:

```bash
orbit task add \
  --title "<title>" \
  --description "<multi-line markdown>" \
  --plan "<multi-line markdown>" \
  --context "<comma,separated,context>" \
  --workspace "<absolute_or_relative_repo_path>" \
  --assigned-to "<identity_display_name>" \
  --created-by "<identity_display_name>" \
  --priority <low|medium|high|critical> \
  --type issue \
  --proposed-by "<identity_display_name>"
```


Include relevant details from the job-run output in the task description.

### 4. Write up a Summary Report

Provide a summary overview of the job-runs in a markdown format, and write the report to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD/operation_<title>.md`.

After writing the report file, return the following JSON result:

```json
{
  "comment": "<one-line summary of operations reviewed>",
  "created_file": "<repo-relative or absolute path to the report file>"
}
```

Orbit will auto-commit the report file together with the run artifact. Do **not** include a `commit` field — Orbit handles the commit.

---

# Operational Guidelines

- Do not modify jobs automatically.
- Do not retry runs automatically.
- Focus only on detection and issue creation.
- Always create one task per failed job-run.

---

# Expected Outcome

After execution:
- All successful job runs are archived. 
- All failed job runs are inspected.
- A corresponding Orbit task exists for each failure.
- Operations maintain clear visibility into runtime issues.
