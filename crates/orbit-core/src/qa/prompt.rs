//! Composition of the QA agent prompt [ORB-10146].
//!
//! The prompt is the contract between the sweep and the QA agent: it carries
//! the workspace, branch, and `baseline..head` commit range, and instructs the
//! agent to identify new features / behaviour changes, exercise them hands-on
//! (not just re-run the test suite), and emit a structured findings report as
//! its final output. Task filing stays in the sweep — the agent only reports.

/// Inputs to the QA prompt for one workspace pass.
pub(crate) struct PromptInputs<'a> {
    pub workspace: &'a str,
    pub repo_root: &'a str,
    pub branch: &'a str,
    pub baseline: Option<&'a str>,
    pub head: &'a str,
    pub watermark_reset: bool,
    /// Commit summaries in `baseline..head`, newest first (already capped).
    pub commits: &'a [String],
}

/// Build the QA agent prompt from the pass inputs.
pub(crate) fn compose_prompt(inputs: &PromptInputs<'_>) -> String {
    let range = match (inputs.baseline, inputs.watermark_reset) {
        (Some(baseline), false) => format!("{baseline}..{}", inputs.head),
        (Some(baseline), true) => format!(
            "history was rewritten since last validation (last-validated commit {baseline} no \
             longer resolves); treat HEAD {} as newly landed",
            inputs.head
        ),
        (None, _) => format!(
            "first validation of this workspace; treat HEAD {} as newly landed",
            inputs.head
        ),
    };

    let commits = if inputs.commits.is_empty() {
        "(commit list unavailable — inspect the range with git directly)".to_string()
    } else {
        inputs
            .commits
            .iter()
            .map(|commit| format!("- {commit}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "You are a QA engineer validating newly-landed work on a direct-push branch. \
         Your job is to confirm that the NEW features and behaviour changes in this range \
         actually work — not merely that the build is green.\n\n\
         ## Target\n\
         - workspace: {workspace}\n\
         - repository root: {repo_root}\n\
         - branch: {branch}\n\
         - commit range: {range}\n\n\
         ## Commits since last validation (newest first)\n\
         {commits}\n\n\
         ## What to do\n\
         1. Read the diff for this range (`git diff {baseline_arg}`, `git log -p`, etc.) and \
         identify the new features and behaviour changes it introduces.\n\
         2. Validate each one HANDS-ON from {repo_root}: build it, run it, invoke the relevant \
         CLI/API, drive the changed code path, and add targeted checks. Do not stop at re-running \
         the existing test suite — exercise the new behaviour the way a user would.\n\
         3. Record concrete issues you find: regressions, features that do not work as intended, \
         broken behaviour changes, missing wiring. Ignore pre-existing issues unrelated to this \
         range.\n\n\
         ## Required final output\n\
         End your run with a single JSON document — and nothing after it — of exactly this shape:\n\n\
         ```json\n\
         {{\"findings\": [\n\
         \x20 {{\n\
         \x20   \"name\": \"short stable slug for the issue\",\n\
         \x20   \"severity\": \"critical|high|medium|low\",\n\
         \x20   \"summary\": \"one-line description\",\n\
         \x20   \"evidence\": \"how you reproduced it: commands, output, reasoning\",\n\
         \x20   \"commits\": [\"<sha> subject\"]\n\
         \x20 }}\n\
         ]}}\n\
         ```\n\n\
         Use an EMPTY array (`{{\"findings\": []}}`) when the new work validates cleanly. \
         Keep each `name` stable and specific so the same issue re-reported on a later sweep is \
         recognizable. Do NOT file tasks yourself — reporting the JSON is your only deliverable.",
        workspace = inputs.workspace,
        repo_root = inputs.repo_root,
        branch = inputs.branch,
        range = range,
        commits = commits,
        baseline_arg = inputs
            .baseline
            .filter(|_| !inputs.watermark_reset)
            .map(|baseline| format!("{baseline}..{}", inputs.head))
            .unwrap_or_else(|| inputs.head.to_string()),
    )
}
