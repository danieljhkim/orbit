## Context
`task_epic_pipeline` exits from deterministic `load_epic` snapshots, while normal child shipment workflows stop successful subtasks in `review` for human handoff. Treating `review` as open work made a clean epic cycle redispatch already-shipped subtasks or run until its iteration ceiling.

## Decision
For epic orchestration only, treat `review` as a shipped stop state: `load_epic` omits review subtasks from the open workset, allows them to satisfy `all_terminal`, and maps their epic summary state to `done` while preserving the raw task status.

## Consequences
- Epic loops can converge after normal PR/local child shipment without embedding human approval into the pipeline.
- Operators can still inspect raw `status: "review"` in the final snapshot and task records before approving lifecycle completion.
- Cost: `summarize_epic`'s `done` counter now includes review-shipped subtasks for epic completion, so readers must distinguish pipeline completion from task approval.
