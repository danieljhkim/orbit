//! Live price-coverage scan over the invocation store (ORB-10354).
//!
//! The curated coverage guard in `orbit-common`'s pricing tests asserts that a
//! hand-maintained list of fleet model strings prices; it cannot notice a model
//! the fleet started running but nobody added to the list. This scan asks the
//! store instead: group the observed `model` values and report the ones no
//! price row covers.
//!
//! It became possible only once the `model` column stopped carrying
//! unversioned crew aliases — aliases are deliberately unpriced (ADR-0245), so
//! before that fix every scan would have reported `opus`/`sonnet`/`fable` as
//! uncovered and the signal would have been noise. Alias provenance now lives
//! in `model_alias`, which this scan ignores.

use chrono::{DateTime, Utc};

use orbit_common::types::{OrbitError, TokenUsage, derive_cost_usd};

use crate::Store;

use super::types::UnpricedModelRow;

/// A nonzero split so a legitimately zero-rate price row still resolves to
/// `Some` — this scan reports missing coverage, not a cost figure.
fn probe_usage() -> TokenUsage {
    TokenUsage {
        input: 1_000,
        output: 1_000,
        ..TokenUsage::default()
    }
}

impl Store {
    /// Every distinct `invocations.model` with no covering price row, newest
    /// observation first.
    ///
    /// Coverage is probed at each model's most recent observation: a model
    /// whose price row starts partway through its history is *covered* here,
    /// since the gap is a historical rate boundary rather than a missing rate.
    /// Rows with a NULL model (an alias Orbit could not resolve) are skipped —
    /// they have no model string to price.
    pub fn list_unpriced_invocation_models(&self) -> Result<Vec<UnpricedModelRow>, OrbitError> {
        let conn = self.read()?;
        let mut stmt = conn
            .prepare(
                "SELECT model, COUNT(*), MIN(ts), MAX(ts) FROM invocations \
                 WHERE model IS NOT NULL AND TRIM(model) <> '' \
                 GROUP BY model ORDER BY MAX(ts) DESC, model ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let usage = probe_usage();
        let mut unpriced = Vec::new();
        for row in rows {
            let (model, invocation_count, first_seen, last_seen) =
                row.map_err(|e| OrbitError::Store(e.to_string()))?;
            let Some(probe_at) = parse_timestamp(&last_seen) else {
                // An unparseable timestamp is a separate data-quality problem;
                // don't turn it into a phantom pricing gap.
                continue;
            };
            if derive_cost_usd(&model, probe_at, &usage).is_some() {
                continue;
            }
            unpriced.push(UnpricedModelRow {
                model,
                invocation_count,
                first_seen,
                last_seen,
            });
        }
        Ok(unpriced)
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
