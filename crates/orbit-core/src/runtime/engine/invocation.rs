use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use orbit_store::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationAccountingFact,
    InvocationAccountingQuery, InvocationInsertParams, InvocationQuery, InvocationRecord, Store,
    TaskInvocationMetrics, ToolInvocationMetrics,
};
use serde::{Deserialize, Serialize};

use crate::OrbitRuntime;

/// Conservative ownership class for managed invocation accounting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorMetricsBucketKind {
    Missing,
    Unattributed,
    Orchestrator,
    Shared,
}

/// Reconciliation fields for one ownership bucket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestratorInvocationMetricsBucket {
    pub kind: OrchestratorMetricsBucketKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<String>,
    pub invocation_count: u64,
    pub linked_task_count: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_create_tokens: u64,
    pub cache_create_1h_tokens: u64,
    pub output_tokens: u64,
    pub provider_cost_usd: f64,
    pub provider_cost_count: u64,
    pub derived_cost_usd: f64,
    pub derived_cost_count: u64,
    pub comparable_provider_cost_usd: f64,
    pub comparable_derived_cost_usd: f64,
    pub comparable_cost_count: u64,
    pub comparable_cost_delta_usd: f64,
    pub missing_provider_count: u64,
    pub unpriced_derived_count: u64,
}

/// Stable, managed-execution-only accounting snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestratorInvocationMetrics {
    pub as_of: DateTime<Utc>,
    pub since: Option<DateTime<Utc>>,
    /// Effective exclusive upper cutoff (never later than `as_of`).
    pub until: DateTime<Utc>,
    pub buckets: Vec<OrchestratorInvocationMetricsBucket>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BucketKey {
    kind: OrchestratorMetricsBucketKind,
    orchestrator: Option<String>,
}

#[derive(Debug, Default)]
struct BucketAccumulator {
    linked_task_ids: BTreeSet<String>,
    invocation_count: u64,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_create_tokens: u64,
    cache_create_1h_tokens: u64,
    output_tokens: u64,
    provider_cost_usd: f64,
    provider_cost_count: u64,
    derived_cost_usd: f64,
    derived_cost_count: u64,
    comparable_provider_cost_usd: f64,
    comparable_derived_cost_usd: f64,
    comparable_cost_count: u64,
    missing_provider_count: u64,
    unpriced_derived_count: u64,
}

impl BucketAccumulator {
    fn add(&mut self, fact: &InvocationAccountingFact) {
        self.invocation_count = self.invocation_count.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(fact.input_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(fact.cache_read_tokens);
        self.cache_create_tokens = self
            .cache_create_tokens
            .saturating_add(fact.cache_create_tokens);
        self.cache_create_1h_tokens = self
            .cache_create_1h_tokens
            .saturating_add(fact.cache_create_1h_tokens);
        self.output_tokens = self.output_tokens.saturating_add(fact.output_tokens);
        self.linked_task_ids.extend(fact.task_ids.iter().cloned());

        let provider = fact.provider_cost_usd.filter(|cost| valid_cost(*cost));
        let derived = fact.derived_cost_usd.filter(|cost| valid_cost(*cost));
        match provider {
            Some(cost) => {
                self.provider_cost_usd += cost;
                self.provider_cost_count = self.provider_cost_count.saturating_add(1);
            }
            None => self.missing_provider_count = self.missing_provider_count.saturating_add(1),
        }
        match derived {
            Some(cost) => {
                self.derived_cost_usd += cost;
                self.derived_cost_count = self.derived_cost_count.saturating_add(1);
            }
            None => self.unpriced_derived_count = self.unpriced_derived_count.saturating_add(1),
        }
        if let (Some(provider), Some(derived)) = (provider, derived) {
            self.comparable_provider_cost_usd += provider;
            self.comparable_derived_cost_usd += derived;
            self.comparable_cost_count = self.comparable_cost_count.saturating_add(1);
        }
    }

    fn finish(
        self,
        kind: OrchestratorMetricsBucketKind,
        orchestrator: Option<String>,
    ) -> OrchestratorInvocationMetricsBucket {
        OrchestratorInvocationMetricsBucket {
            kind,
            orchestrator,
            invocation_count: self.invocation_count,
            linked_task_count: self.linked_task_ids.len() as u64,
            input_tokens: self.input_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_create_tokens: self.cache_create_tokens,
            cache_create_1h_tokens: self.cache_create_1h_tokens,
            output_tokens: self.output_tokens,
            provider_cost_usd: self.provider_cost_usd,
            provider_cost_count: self.provider_cost_count,
            derived_cost_usd: self.derived_cost_usd,
            derived_cost_count: self.derived_cost_count,
            comparable_provider_cost_usd: self.comparable_provider_cost_usd,
            comparable_derived_cost_usd: self.comparable_derived_cost_usd,
            comparable_cost_count: self.comparable_cost_count,
            comparable_cost_delta_usd: self.comparable_provider_cost_usd
                - self.comparable_derived_cost_usd,
            missing_provider_count: self.missing_provider_count,
            unpriced_derived_count: self.unpriced_derived_count,
        }
    }
}

impl OrbitRuntime {
    pub fn activity_invocation_metrics(
        &self,
    ) -> Result<Vec<ActivityInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_activity_invocation_metrics()
    }

    pub fn agent_invocation_metrics(&self) -> Result<Vec<AgentInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_agent_invocation_metrics()
    }

    pub fn task_invocation_metrics(
        &self,
        task_id: &str,
    ) -> Result<TaskInvocationMetrics, OrbitError> {
        open_invocation_store(self)?.get_task_invocation_metrics(task_id)
    }

    pub fn tool_invocation_metrics(&self) -> Result<Vec<ToolInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_tool_invocation_metrics()
    }

    pub fn invocation_records(
        &self,
        filter: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        open_invocation_store(self)?.list_invocation_records(&filter)
    }

    pub fn insert_invocation_trace_record(
        &self,
        params: &InvocationInsertParams,
    ) -> Result<(), OrbitError> {
        open_invocation_store(self)?.insert_invocation_trace_record(params)
    }

    /// Aggregates managed invocation telemetry by canonical task orchestrator.
    ///
    /// The effective window is half-open (`since <= ts < until`). Missing
    /// tasks take precedence over unattributed tasks, which take precedence
    /// over named/shared attribution, so incomplete ownership never charges a
    /// known orchestrator.
    pub fn orchestrator_invocation_metrics(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<OrchestratorInvocationMetrics, OrbitError> {
        let as_of = Utc::now();
        if let (Some(since), Some(until)) = (since, until)
            && since >= until
        {
            return Err(OrbitError::InvalidInput(
                "since must be earlier than until".to_string(),
            ));
        }
        let effective_until = until.map_or(as_of, |until| until.min(as_of));
        if since.is_some_and(|since| since >= effective_until) {
            return Err(OrbitError::InvalidInput(
                "since must be earlier than the effective upper cutoff".to_string(),
            ));
        }

        let facts = open_invocation_store(self)?.list_invocation_accounting_facts(
            &InvocationAccountingQuery {
                since,
                until: effective_until,
            },
        )?;
        let task_orchestrators = self
            .list_tasks()?
            .into_iter()
            .map(|task| (task.id.to_string(), task.orchestrator))
            .collect::<HashMap<_, _>>();

        let mut grouped = BTreeMap::<BucketKey, BucketAccumulator>::new();
        for fact in &facts {
            let key = classify_invocation(&fact.task_ids, &task_orchestrators);
            grouped.entry(key).or_default().add(fact);
        }
        let buckets = grouped
            .into_iter()
            .map(|(key, bucket)| bucket.finish(key.kind, key.orchestrator))
            .collect();

        Ok(OrchestratorInvocationMetrics {
            as_of,
            since,
            until: effective_until,
            buckets,
        })
    }
}

fn classify_invocation(
    task_ids: &[String],
    task_orchestrators: &HashMap<String, Option<String>>,
) -> BucketKey {
    if task_ids.is_empty() {
        return BucketKey {
            kind: OrchestratorMetricsBucketKind::Unattributed,
            orchestrator: None,
        };
    }

    let distinct_task_ids = task_ids.iter().collect::<BTreeSet<_>>();
    if distinct_task_ids
        .iter()
        .any(|task_id| !task_orchestrators.contains_key(task_id.as_str()))
    {
        return BucketKey {
            kind: OrchestratorMetricsBucketKind::Missing,
            orchestrator: None,
        };
    }

    let owners = distinct_task_ids
        .iter()
        .filter_map(|task_id| task_orchestrators.get(task_id.as_str()))
        .collect::<Vec<_>>();
    if owners.iter().any(|owner| owner.is_none()) {
        return BucketKey {
            kind: OrchestratorMetricsBucketKind::Unattributed,
            orchestrator: None,
        };
    }
    let named = owners
        .into_iter()
        .filter_map(|owner| owner.as_ref())
        .collect::<BTreeSet<_>>();
    if named.len() == 1 {
        BucketKey {
            kind: OrchestratorMetricsBucketKind::Orchestrator,
            orchestrator: named.into_iter().next().cloned(),
        }
    } else {
        BucketKey {
            kind: OrchestratorMetricsBucketKind::Shared,
            orchestrator: None,
        }
    }
}

fn valid_cost(cost: f64) -> bool {
    cost.is_finite() && cost >= 0.0
}

fn open_invocation_store(runtime: &OrbitRuntime) -> Result<Store, OrbitError> {
    runtime.sqlite_store()
}
