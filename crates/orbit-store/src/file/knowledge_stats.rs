use std::cmp::Ordering;

use orbit_types::JobRun;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RatioSummary {
    pub mean: f64,
    pub p50: f64,
    pub p90: f64,
    pub min: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoubleReadSummary {
    pub mean_rate: f64,
    pub runs_over_fifty_percent: u64,
    pub measured_runs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenInputSummary {
    pub with_pack_avg: f64,
    pub without_pack_avg: f64,
    pub estimated_savings: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeStatsSummary {
    pub total_runs: u64,
    pub pack_runs: u64,
    pub fallback_runs: u64,
    pub fallback_rate: f64,
    pub compression: Option<RatioSummary>,
    pub double_read: DoubleReadSummary,
    pub total_llm_input_tokens: TokenInputSummary,
}

pub fn aggregate(runs: &[JobRun]) -> KnowledgeStatsSummary {
    let metrics = runs
        .iter()
        .filter_map(|run| run.knowledge_metrics.as_ref())
        .collect::<Vec<_>>();

    let total_runs = metrics.len() as u64;
    let pack_runs = metrics.iter().filter(|m| m.knowledge_pack_used).count() as u64;
    let fallback_runs = total_runs.saturating_sub(pack_runs);
    let fallback_rate = ratio(fallback_runs, total_runs).unwrap_or(0.0);

    let compression_values = metrics
        .iter()
        .filter_map(|m| m.compression_ratio)
        .collect::<Vec<_>>();
    let double_read_values = metrics
        .iter()
        .filter_map(|m| m.double_read_rate)
        .collect::<Vec<_>>();
    let with_pack_tokens = metrics
        .iter()
        .filter(|m| m.knowledge_pack_used)
        .map(|m| m.total_llm_input_tokens as f64)
        .collect::<Vec<_>>();
    let without_pack_tokens = metrics
        .iter()
        .filter(|m| !m.knowledge_pack_used)
        .map(|m| m.total_llm_input_tokens as f64)
        .collect::<Vec<_>>();

    KnowledgeStatsSummary {
        total_runs,
        pack_runs,
        fallback_runs,
        fallback_rate,
        compression: summarize_ratios(&compression_values),
        double_read: DoubleReadSummary {
            mean_rate: mean(&double_read_values),
            runs_over_fifty_percent: double_read_values
                .iter()
                .filter(|value| **value > 0.5)
                .count() as u64,
            measured_runs: double_read_values.len() as u64,
        },
        total_llm_input_tokens: TokenInputSummary {
            with_pack_avg: mean(&with_pack_tokens),
            without_pack_avg: mean(&without_pack_tokens),
            estimated_savings: estimate_savings(
                mean(&with_pack_tokens),
                mean(&without_pack_tokens),
            ),
        },
    }
}

fn summarize_ratios(values: &[f64]) -> Option<RatioSummary> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    Some(RatioSummary {
        mean: mean(&sorted),
        p50: percentile(&sorted, 50),
        p90: percentile(&sorted, 90),
        min: sorted[0],
    })
}

fn estimate_savings(with_pack_avg: f64, without_pack_avg: f64) -> Option<f64> {
    (without_pack_avg > 0.0).then_some(1.0 - (with_pack_avg / without_pack_avg))
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((pct as f64 / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then_some(numerator as f64 / denominator as f64)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_types::{JobRun, JobRunState, KnowledgeRunMetrics};

    use super::{KnowledgeStatsSummary, aggregate};

    fn run(id: &str, metrics: KnowledgeRunMetrics) -> JobRun {
        JobRun {
            run_id: id.to_string(),
            job_id: "job".to_string(),
            attempt: 1,
            state: JobRunState::Success,
            scheduled_at: Utc::now(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: Utc::now(),
            pid: None,
            pid_start_time: None,
            input: None,
            retry_source_run_id: None,
            knowledge_metrics: Some(metrics),
            steps: vec![],
        }
    }

    #[test]
    fn aggregates_pack_and_fallback_metrics() {
        let runs = vec![
            run(
                "r1",
                KnowledgeRunMetrics {
                    raw_read_token_baseline: 100,
                    knowledge_pack_tokens: Some(25),
                    compression_ratio: Some(4.0),
                    actual_fs_read_tokens_during_run: 10,
                    double_read_rate: Some(0.1),
                    knowledge_pack_used: true,
                    knowledge_pack_unresolved_count: 0,
                    total_llm_input_tokens: 400,
                },
            ),
            run(
                "r2",
                KnowledgeRunMetrics {
                    raw_read_token_baseline: 120,
                    knowledge_pack_tokens: Some(40),
                    compression_ratio: Some(3.0),
                    actual_fs_read_tokens_during_run: 24,
                    double_read_rate: Some(0.2),
                    knowledge_pack_used: true,
                    knowledge_pack_unresolved_count: 1,
                    total_llm_input_tokens: 500,
                },
            ),
            run(
                "r3",
                KnowledgeRunMetrics {
                    raw_read_token_baseline: 130,
                    knowledge_pack_tokens: Some(65),
                    compression_ratio: Some(2.0),
                    actual_fs_read_tokens_during_run: 91,
                    double_read_rate: Some(0.7),
                    knowledge_pack_used: true,
                    knowledge_pack_unresolved_count: 2,
                    total_llm_input_tokens: 700,
                },
            ),
            run(
                "r4",
                KnowledgeRunMetrics {
                    raw_read_token_baseline: 90,
                    knowledge_pack_tokens: None,
                    compression_ratio: None,
                    actual_fs_read_tokens_during_run: 18,
                    double_read_rate: Some(0.2),
                    knowledge_pack_used: false,
                    knowledge_pack_unresolved_count: 0,
                    total_llm_input_tokens: 900,
                },
            ),
            run(
                "r5",
                KnowledgeRunMetrics {
                    raw_read_token_baseline: 80,
                    knowledge_pack_tokens: None,
                    compression_ratio: None,
                    actual_fs_read_tokens_during_run: 24,
                    double_read_rate: Some(0.3),
                    knowledge_pack_used: false,
                    knowledge_pack_unresolved_count: 0,
                    total_llm_input_tokens: 1100,
                },
            ),
        ];

        let summary: KnowledgeStatsSummary = aggregate(&runs);

        assert_eq!(summary.total_runs, 5);
        assert_eq!(summary.pack_runs, 3);
        assert_eq!(summary.fallback_runs, 2);
        assert!((summary.fallback_rate - 0.4).abs() < f64::EPSILON);
        let compression = summary.compression.expect("compression");
        assert!((compression.mean - 3.0).abs() < f64::EPSILON);
        assert!((compression.p50 - 3.0).abs() < f64::EPSILON);
        assert!((compression.p90 - 4.0).abs() < f64::EPSILON);
        assert!((compression.min - 2.0).abs() < f64::EPSILON);
        assert!(
            (summary.double_read.mean_rate - ((0.1 + 0.2 + 0.7 + 0.2 + 0.3) / 5.0)).abs() < 1e-9
        );
        assert_eq!(summary.double_read.runs_over_fifty_percent, 1);
        assert_eq!(summary.double_read.measured_runs, 5);
        assert!(
            (summary.total_llm_input_tokens.with_pack_avg - ((400.0 + 500.0 + 700.0) / 3.0)).abs()
                < 1e-9
        );
        assert!((summary.total_llm_input_tokens.without_pack_avg - 1000.0).abs() < 1e-9);
        assert!(summary.total_llm_input_tokens.estimated_savings.is_some());
    }
}
