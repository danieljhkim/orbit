//! Dormant hub-global ADR/learning sequence persistence [ORB-10272].

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION, HubKnowledgeAllocationRequestV1,
    HubKnowledgeAllocationV1, KnowledgeIdKind, McpCapability, McpLeasedRun, McpTransport,
    OrbitError,
};
use orbit_store::AuditEventInsertParams;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::RemoteStore;

type KnowledgeSourcesByWorkspace = Vec<(String, BTreeSet<String>)>;
type GlobalKnowledgeInventory = BTreeMap<(KnowledgeIdKind, String), KnowledgeSourcesByWorkspace>;
type NormalizedKnowledgeInventory =
    BTreeMap<(KnowledgeIdKind, u32), Vec<(String, String, BTreeSet<String>)>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubKnowledgeAllocatorStatus {
    Dormant,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnowledgeAuthorityCutoverStatus {
    PreActivation,
    Reconciling,
    Active,
    FailedIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeAuthorityCutoverState {
    pub status: KnowledgeAuthorityCutoverStatus,
    pub generation: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubKnowledgeAllocatorState {
    pub status: HubKnowledgeAllocatorStatus,
    pub activation_generation: u64,
    pub activated_at: Option<DateTime<Utc>>,
    pub adr_next_sequence: u64,
    pub learning_next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LegacyKnowledgeId {
    pub kind: KnowledgeIdKind,
    pub id: String,
    pub evidence: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeWorkspaceInventory {
    pub workspace_id: String,
    pub ids: Vec<LegacyKnowledgeId>,
}

impl KnowledgeWorkspaceInventory {
    pub fn validated(mut self) -> Result<Self, OrbitError> {
        orbit_common::types::validate_registry_identifier("workspace_id", &self.workspace_id)?;
        let mut merged: BTreeMap<(KnowledgeIdKind, String), BTreeSet<String>> = BTreeMap::new();
        for record in self.ids {
            let sequence = record.kind.parse_id(&record.id).ok_or_else(|| {
                OrbitError::Migration(format!(
                    "workspace '{}' contains invalid {} id '{}' in legacy knowledge inventory",
                    self.workspace_id,
                    record.kind.as_str(),
                    record.id
                ))
            })?;
            if sequence == 0
                || record.evidence.is_empty()
                || record
                    .evidence
                    .iter()
                    .any(|value| value.trim().is_empty() || value.trim() != value)
            {
                return Err(OrbitError::Migration(format!(
                    "workspace '{}' {} id '{}' has invalid sequence or source evidence",
                    self.workspace_id,
                    record.kind.as_str(),
                    record.id
                )));
            }
            merged
                .entry((record.kind, record.id))
                .or_default()
                .extend(record.evidence);
        }
        let mut normalized_ids = NormalizedKnowledgeInventory::new();
        for ((kind, id), evidence) in &merged {
            let sequence = kind.parse_id(id).ok_or_else(|| {
                OrbitError::Migration(format!(
                    "workspace '{}' contains invalid {} id '{}' after inventory normalization",
                    self.workspace_id,
                    kind.as_str(),
                    id
                ))
            })?;
            normalized_ids.entry((*kind, sequence)).or_default().push((
                self.workspace_id.clone(),
                id.clone(),
                evidence.clone(),
            ));
        }
        let conflicts = normalized_sequence_conflicts(normalized_ids);
        if !conflicts.is_empty() {
            return Err(OrbitError::Migration(format!(
                "workspace '{}' contains numerically duplicate knowledge identifiers:\n- {}",
                self.workspace_id,
                conflicts.join("\n- ")
            )));
        }
        self.ids = merged
            .into_iter()
            .map(|((kind, id), evidence)| LegacyKnowledgeId { kind, id, evidence })
            .collect();
        Ok(self)
    }

    fn source_digest(&self) -> Result<String, OrbitError> {
        let payload = serde_json::to_vec(&self.ids).map_err(|error| {
            OrbitError::Store(format!("serialize knowledge inventory: {error}"))
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(payload)))
    }

    fn maximum(&self, kind: KnowledgeIdKind) -> u32 {
        self.ids
            .iter()
            .filter(|record| record.kind == kind)
            .filter_map(|record| kind.parse_id(&record.id))
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubKnowledgeRequestIdentityV1 {
    pub schema_version: u8,
    pub workspace_id: String,
    pub kind: KnowledgeIdKind,
    pub model: Option<String>,
    pub caller_machine_id: String,
    pub caller_host_id: String,
    pub process_machine_id: String,
    pub process_host_id: String,
    pub transport: McpTransport,
    pub effective_capabilities: BTreeSet<McpCapability>,
    pub origin_session_id: String,
    pub mcp_call_id: String,
    pub leased_run: Option<McpLeasedRun>,
}

impl RemoteStore {
    pub fn knowledge_allocator_state(&self) -> Result<HubKnowledgeAllocatorState, OrbitError> {
        self.read(read_allocator_state)
    }

    pub(crate) fn knowledge_cutover_state(
        &self,
    ) -> Result<KnowledgeAuthorityCutoverState, OrbitError> {
        self.read(read_cutover_state)
    }

    /// Enter (or resume) reconciliation before source discovery. Managed
    /// authoring consults the hub before this point, so retrying an interrupted
    /// generation cannot expose the legacy allocator as a second authority.
    pub(crate) fn begin_knowledge_cutover(
        &self,
    ) -> Result<KnowledgeAuthorityCutoverState, OrbitError> {
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let current = read_cutover_state(tx.connection())?;
                if current.status == KnowledgeAuthorityCutoverStatus::Active {
                    return Ok(current);
                }
                let generation = current.generation.checked_add(1).ok_or_else(|| {
                    OrbitError::Store("knowledge cutover generation overflow".into())
                })?;
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_cutover_state
                         SET status = 'reconciling', generation = ?1,
                             last_error = NULL, updated_at = ?2 WHERE id = 0",
                        params![
                            u64_to_i64(generation, "knowledge cutover generation")?,
                            super::now_string()
                        ],
                    )
                    .map_err(|error| {
                        OrbitError::Store(format!("begin knowledge authority cutover: {error}"))
                    })?;
                read_cutover_state(tx.connection())
            })
    }

    pub(crate) fn complete_knowledge_cutover(
        &self,
    ) -> Result<KnowledgeAuthorityCutoverState, OrbitError> {
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let allocator = read_allocator_state(tx.connection())?;
                if allocator.status != HubKnowledgeAllocatorStatus::Active {
                    return Err(OrbitError::Store(
                        "cannot complete knowledge cutover before allocator activation".into(),
                    ));
                }
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_cutover_state
                         SET status = 'active', generation = MAX(generation, ?1),
                             last_error = NULL, updated_at = ?2 WHERE id = 0",
                        params![
                            u64_to_i64(
                                allocator.activation_generation,
                                "knowledge activation generation"
                            )?,
                            super::now_string()
                        ],
                    )
                    .map_err(|error| {
                        OrbitError::Store(format!("complete knowledge authority cutover: {error}"))
                    })?;
                read_cutover_state(tx.connection())
            })
    }

    pub(crate) fn fail_knowledge_cutover(
        &self,
        error: &OrbitError,
    ) -> Result<KnowledgeAuthorityCutoverState, OrbitError> {
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let current = read_cutover_state(tx.connection())?;
                if current.status == KnowledgeAuthorityCutoverStatus::Active {
                    return Ok(current);
                }
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_cutover_state
                         SET status = 'failed-incomplete', last_error = ?1,
                             updated_at = ?2 WHERE id = 0",
                        params![error.to_string(), super::now_string()],
                    )
                    .map_err(|store_error| {
                        OrbitError::Store(format!(
                            "record failed knowledge authority cutover: {store_error}"
                        ))
                    })?;
                read_cutover_state(tx.connection())
            })
    }

    pub(crate) fn activate_knowledge_allocator(
        &self,
        inventories: Vec<KnowledgeWorkspaceInventory>,
    ) -> Result<HubKnowledgeAllocatorState, OrbitError> {
        let inventories = validate_inventory_set(inventories)?;
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let current = read_allocator_state(tx.connection())?;
                if current.status == HubKnowledgeAllocatorStatus::Active {
                    ensure_inventories_already_reconciled(tx.connection(), &inventories)?;
                    return read_allocator_state(tx.connection());
                }
                let generation = current
                    .activation_generation
                    .checked_add(1)
                    .ok_or_else(|| {
                        OrbitError::Store("knowledge activation generation overflow".into())
                    })?;
                for inventory in &inventories {
                    apply_inventory(tx.connection(), inventory, generation)?;
                }
                seed_sequences_from_ids(tx.connection())?;
                let now = super::now_string();
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_allocator_state
                         SET status = 'active', activation_generation = ?1,
                             activated_at = ?2, updated_at = ?2
                         WHERE id = 0 AND status = 'dormant'",
                        params![u64_to_i64(generation, "activation generation")?, now],
                    )
                    .map_err(|error| {
                        OrbitError::Store(format!("activate knowledge allocator: {error}"))
                    })?;
                read_allocator_state(tx.connection())
            })
    }

    // F1 deliberately installs a dormant substrate; F3 will call this for late workspaces.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn reconcile_knowledge_workspace(
        &self,
        inventory: KnowledgeWorkspaceInventory,
    ) -> Result<HubKnowledgeAllocatorState, OrbitError> {
        let inventory = inventory.validated()?;
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let current = read_allocator_state(tx.connection())?;
                if current.status != HubKnowledgeAllocatorStatus::Active {
                    return Err(OrbitError::InvalidInput(
                        "hub knowledge allocator is dormant; activate it before reconciling a late workspace"
                            .to_string(),
                    ));
                }
                let generation = current
                    .activation_generation
                    .checked_add(1)
                    .ok_or_else(|| OrbitError::Store("knowledge reconciliation generation overflow".into()))?;
                apply_inventory(tx.connection(), &inventory, generation)?;
                seed_sequences_from_ids(tx.connection())?;
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_allocator_state
                         SET activation_generation = ?1, updated_at = ?2 WHERE id = 0",
                        params![u64_to_i64(generation, "reconciliation generation")?, super::now_string()],
                    )
                    .map_err(|error| OrbitError::Store(format!("advance reconciliation generation: {error}")))?;
                read_allocator_state(tx.connection())
            })
    }

    pub(crate) fn allocate_hub_knowledge_id(
        &self,
        request: &HubKnowledgeAllocationRequestV1,
        identity: &HubKnowledgeRequestIdentityV1,
        audit: &AuditEventInsertParams,
    ) -> Result<HubKnowledgeAllocationV1, OrbitError> {
        request.validate()?;
        validate_request_identity(request, identity, audit)?;
        let identity_json = serde_json::to_string(identity).map_err(|error| {
            OrbitError::Store(format!("serialize hub knowledge request identity: {error}"))
        })?;

        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                if let Some((allocation, stored_identity)) =
                    allocation_by_call(tx.connection(), &identity.mcp_call_id)?
                {
                    if stored_identity == identity_json {
                        return Ok(allocation);
                    }
                    return Err(OrbitError::InvalidInput(format!(
                        "mcp_call_id '{}' was already used for a different hub knowledge allocation request",
                        identity.mcp_call_id
                    )));
                }
                if read_allocator_state(tx.connection())?.status
                    != HubKnowledgeAllocatorStatus::Active
                {
                    return Err(OrbitError::InvalidInput(
                        "hub knowledge allocator is dormant".to_string(),
                    ));
                }
                let eligible: bool = tx
                    .connection()
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM hub_knowledge_workspace_reconciliation
                            WHERE workspace_id = ?1
                         )",
                        [&request.workspace_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                if !eligible {
                    return Err(OrbitError::InvalidInput(format!(
                        "workspace '{}' is not eligible for hub knowledge allocation until its legacy sources are reconciled",
                        request.workspace_id
                    )));
                }

                let next_raw: i64 = tx
                    .connection()
                    .query_row(
                        "SELECT next_sequence FROM hub_knowledge_sequences WHERE kind = ?1",
                        [request.kind.as_str()],
                        |row| row.get(0),
                    )
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                let sequence = u32::try_from(next_raw).map_err(|_| {
                    OrbitError::Execution(format!(
                        "{} knowledge id sequence is exhausted",
                        request.kind.as_str()
                    ))
                })?;
                let id = request.kind.format_id(sequence);
                let allocated_at = Utc::now();
                tx.connection()
                    .execute(
                        "INSERT INTO hub_knowledge_ids(
                            kind, id, workspace_id, sequence, origin, evidence_json, recorded_at
                         ) VALUES (?1, ?2, ?3, ?4, 'allocated', '[]', ?5)",
                        params![
                            request.kind.as_str(), id, request.workspace_id,
                            i64::from(sequence), allocated_at.to_rfc3339(),
                        ],
                    )
                    .map_err(|error| OrbitError::Store(format!("record allocated knowledge id: {error}")))?;
                tx.connection()
                    .execute(
                        "INSERT INTO hub_knowledge_allocation_ledger(
                            mcp_call_id, workspace_id, kind, id, sequence,
                            request_identity_json, allocated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            identity.mcp_call_id, request.workspace_id, request.kind.as_str(), id,
                            i64::from(sequence), identity_json, allocated_at.to_rfc3339(),
                        ],
                    )
                    .map_err(|error| OrbitError::Store(format!("record knowledge allocation ledger: {error}")))?;
                tx.connection()
                    .execute(
                        "UPDATE hub_knowledge_sequences
                         SET next_sequence = ?2, updated_at = ?3 WHERE kind = ?1",
                        params![
                            request.kind.as_str(), i64::from(sequence) + 1,
                            allocated_at.to_rfc3339(),
                        ],
                    )
                    .map_err(|error| OrbitError::Store(format!("advance knowledge sequence: {error}")))?;
                let mut audit = audit.clone();
                audit.target_id = Some(id.clone());
                tx.insert_audit_event_record(&audit)?;
                Ok(HubKnowledgeAllocationV1 {
                    schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
                    workspace_id: request.workspace_id.clone(),
                    kind: request.kind,
                    id,
                    sequence,
                    mcp_call_id: identity.mcp_call_id.clone(),
                    allocated_at,
                })
            })
    }

    pub(crate) fn hub_knowledge_allocation_by_call(
        &self,
        mcp_call_id: &str,
    ) -> Result<Option<HubKnowledgeAllocationV1>, OrbitError> {
        validate_correlation(mcp_call_id)?;
        self.read(|conn| Ok(allocation_by_call(conn, mcp_call_id)?.map(|row| row.0)))
    }

    pub(crate) fn hub_knowledge_allocation_by_id(
        &self,
        workspace_id: &str,
        kind: KnowledgeIdKind,
        id: &str,
    ) -> Result<Option<HubKnowledgeAllocationV1>, OrbitError> {
        orbit_common::types::validate_registry_identifier("workspace_id", workspace_id)?;
        if kind.parse_id(id).is_none() {
            return Err(OrbitError::InvalidInput(format!(
                "invalid {} id '{id}'",
                kind.as_str()
            )));
        }
        self.read(|conn| {
            conn.query_row(
                "SELECT workspace_id, kind, id, sequence, mcp_call_id, allocated_at
                 FROM hub_knowledge_allocation_ledger
                 WHERE workspace_id = ?1 AND kind = ?2 AND id = ?3",
                params![workspace_id, kind.as_str(), id],
                allocation_row,
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))
        })
    }

    pub(crate) fn hub_knowledge_id_workspace(
        &self,
        kind: KnowledgeIdKind,
        id: &str,
    ) -> Result<Option<String>, OrbitError> {
        if kind.parse_id(id).is_none() {
            return Err(OrbitError::InvalidInput(format!(
                "invalid {} id '{id}'",
                kind.as_str()
            )));
        }
        self.read(|conn| {
            conn.query_row(
                "SELECT workspace_id FROM hub_knowledge_ids WHERE kind = ?1 AND id = ?2",
                params![kind.as_str(), id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))
        })
    }
}

fn validate_inventory_set(
    inventories: Vec<KnowledgeWorkspaceInventory>,
) -> Result<Vec<KnowledgeWorkspaceInventory>, OrbitError> {
    let mut normalized = Vec::with_capacity(inventories.len());
    let mut workspaces = BTreeSet::new();
    let mut global_ids = GlobalKnowledgeInventory::new();
    let mut normalized_ids = NormalizedKnowledgeInventory::new();
    for inventory in inventories {
        let inventory = inventory.validated()?;
        if !workspaces.insert(inventory.workspace_id.clone()) {
            return Err(OrbitError::Migration(format!(
                "knowledge activation inventory repeats workspace '{}'",
                inventory.workspace_id
            )));
        }
        for record in &inventory.ids {
            let sequence = record.kind.parse_id(&record.id).ok_or_else(|| {
                OrbitError::Migration(format!(
                    "workspace '{}' contains invalid {} id '{}' after inventory validation",
                    inventory.workspace_id,
                    record.kind.as_str(),
                    record.id
                ))
            })?;
            global_ids
                .entry((record.kind, record.id.clone()))
                .or_default()
                .push((inventory.workspace_id.clone(), record.evidence.clone()));
            normalized_ids
                .entry((record.kind, sequence))
                .or_default()
                .push((
                    inventory.workspace_id.clone(),
                    record.id.clone(),
                    record.evidence.clone(),
                ));
        }
        normalized.push(inventory);
    }
    let mut conflicts = global_ids
        .into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|((kind, id), sources)| {
            let sources = sources
                .into_iter()
                .map(|(workspace, evidence)| {
                    format!(
                        "{} [{}]",
                        workspace,
                        evidence.into_iter().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("{} {id}: {sources}", kind.as_str())
        })
        .collect::<Vec<_>>();
    conflicts.extend(normalized_sequence_conflicts(normalized_ids));
    if !conflicts.is_empty() {
        return Err(OrbitError::Migration(format!(
            "duplicate global knowledge identifiers were found across registered workspaces:\n- {}",
            conflicts.join("\n- ")
        )));
    }
    normalized.sort_by(|left, right| left.workspace_id.cmp(&right.workspace_id));
    Ok(normalized)
}

fn normalized_sequence_conflicts(inventory: NormalizedKnowledgeInventory) -> Vec<String> {
    inventory
        .into_iter()
        .filter(|(_, sources)| {
            sources
                .iter()
                .map(|(_, id, _)| id)
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
        .map(|((kind, sequence), sources)| {
            let sources = sources
                .into_iter()
                .map(|(workspace, id, evidence)| {
                    format!(
                        "{workspace}/{id} [{}]",
                        evidence.into_iter().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{} sequence {sequence} has multiple textual forms: {sources}",
                kind.as_str()
            )
        })
        .collect()
}

fn apply_inventory(
    conn: &Connection,
    inventory: &KnowledgeWorkspaceInventory,
    generation: u64,
) -> Result<(), OrbitError> {
    let mut conflicts = Vec::new();
    for record in &inventory.ids {
        let sequence = record.kind.parse_id(&record.id).ok_or_else(|| {
            OrbitError::Migration(format!(
                "workspace '{}' contains invalid {} id '{}' after inventory validation",
                inventory.workspace_id,
                record.kind.as_str(),
                record.id
            ))
        })?;
        if let Some((existing_id, workspace_id)) = conn
            .query_row(
                "SELECT id, workspace_id FROM hub_knowledge_ids
                 WHERE kind = ?1 AND (id = ?2 OR sequence = ?3)",
                params![record.kind.as_str(), record.id, i64::from(sequence)],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))?
            .filter(|(existing_id, workspace_id)| {
                workspace_id != &inventory.workspace_id || existing_id != &record.id
            })
        {
            conflicts.push(format!(
                "{} sequence {}: {}/{} [{}]; {}/{} [{}]",
                record.kind.as_str(),
                sequence,
                workspace_id,
                existing_id,
                existing_evidence(conn, record.kind, &existing_id)?.join(", "),
                inventory.workspace_id,
                record.id,
                record
                    .evidence
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if !conflicts.is_empty() {
        return Err(OrbitError::Migration(format!(
            "duplicate global knowledge identifiers were found while reconciling workspace '{}':\n- {}",
            inventory.workspace_id,
            conflicts.join("\n- ")
        )));
    }
    for record in &inventory.ids {
        let sequence = record.kind.parse_id(&record.id).ok_or_else(|| {
            OrbitError::Migration(format!(
                "invalid {} id '{}'",
                record.kind.as_str(),
                record.id
            ))
        })?;
        if let Some((workspace_id, origin)) = conn
            .query_row(
                "SELECT workspace_id, origin FROM hub_knowledge_ids WHERE kind = ?1 AND id = ?2",
                params![record.kind.as_str(), record.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))?
        {
            if workspace_id != inventory.workspace_id {
                return Err(OrbitError::Migration(format!(
                    "duplicate global {} id '{}' appears in reconciled workspace '{}' and workspace '{}'",
                    record.kind.as_str(),
                    record.id,
                    workspace_id,
                    inventory.workspace_id
                )));
            }
            if origin == "legacy" {
                let evidence_json = serde_json::to_string(&record.evidence)
                    .map_err(|error| OrbitError::Store(error.to_string()))?;
                conn.execute(
                    "UPDATE hub_knowledge_ids SET evidence_json = ?3
                     WHERE kind = ?1 AND id = ?2 AND origin = 'legacy'",
                    params![record.kind.as_str(), record.id, evidence_json],
                )
                .map_err(|error| {
                    OrbitError::Store(format!("refresh legacy knowledge evidence: {error}"))
                })?;
            }
            continue;
        }
        let evidence_json = serde_json::to_string(&record.evidence)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        conn.execute(
            "INSERT INTO hub_knowledge_ids(
                kind, id, workspace_id, sequence, origin, evidence_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, 'legacy', ?5, ?6)",
            params![
                record.kind.as_str(),
                record.id,
                inventory.workspace_id,
                i64::from(sequence),
                evidence_json,
                super::now_string(),
            ],
        )
        .map_err(|error| OrbitError::Store(format!("record legacy knowledge id: {error}")))?;
    }
    conn.execute(
        "INSERT INTO hub_knowledge_workspace_reconciliation(
            workspace_id, source_digest, source_count, adr_max, learning_max,
            reconciliation_generation, reconciled_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(workspace_id) DO UPDATE SET
            source_digest = excluded.source_digest,
            source_count = excluded.source_count,
            adr_max = MAX(hub_knowledge_workspace_reconciliation.adr_max, excluded.adr_max),
            learning_max = MAX(hub_knowledge_workspace_reconciliation.learning_max, excluded.learning_max),
            reconciliation_generation = excluded.reconciliation_generation,
            reconciled_at = excluded.reconciled_at",
        params![
            inventory.workspace_id, inventory.source_digest()?,
            i64::try_from(inventory.ids.len()).map_err(|error| OrbitError::Store(error.to_string()))?,
            i64::from(inventory.maximum(KnowledgeIdKind::Adr)),
            i64::from(inventory.maximum(KnowledgeIdKind::Learning)),
            u64_to_i64(generation, "reconciliation generation")?, super::now_string(),
        ],
    )
    .map_err(|error| OrbitError::Store(format!("record workspace knowledge reconciliation: {error}")))?;
    Ok(())
}

fn existing_evidence(
    conn: &Connection,
    kind: KnowledgeIdKind,
    id: &str,
) -> Result<Vec<String>, OrbitError> {
    let raw: String = conn
        .query_row(
            "SELECT evidence_json FROM hub_knowledge_ids WHERE kind = ?1 AND id = ?2",
            params![kind.as_str(), id],
            |row| row.get(0),
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    serde_json::from_str::<BTreeSet<String>>(&raw)
        .map(|evidence| evidence.into_iter().collect())
        .map_err(|error| OrbitError::Store(format!("decode knowledge evidence: {error}")))
}

fn seed_sequences_from_ids(conn: &Connection) -> Result<(), OrbitError> {
    for kind in [KnowledgeIdKind::Adr, KnowledgeIdKind::Learning] {
        let maximum: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM hub_knowledge_ids WHERE kind = ?1",
                [kind.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let next = maximum.checked_add(1).ok_or_else(|| {
            OrbitError::Store(format!("{} knowledge sequence overflow", kind.as_str()))
        })?;
        conn.execute(
            "UPDATE hub_knowledge_sequences
             SET next_sequence = MAX(next_sequence, ?2), updated_at = ?3 WHERE kind = ?1",
            params![kind.as_str(), next, super::now_string()],
        )
        .map_err(|error| {
            OrbitError::Store(format!(
                "seed {} knowledge sequence: {error}",
                kind.as_str()
            ))
        })?;
    }
    Ok(())
}

fn ensure_inventories_already_reconciled(
    conn: &Connection,
    inventories: &[KnowledgeWorkspaceInventory],
) -> Result<(), OrbitError> {
    for inventory in inventories {
        let digest = inventory.source_digest()?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT source_digest FROM hub_knowledge_workspace_reconciliation
                 WHERE workspace_id = ?1",
                [&inventory.workspace_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        if stored.as_deref() != Some(digest.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "hub knowledge allocator is active and workspace '{}' has unreconciled sources",
                inventory.workspace_id
            )));
        }
    }
    Ok(())
}

fn read_allocator_state(conn: &Connection) -> Result<HubKnowledgeAllocatorState, OrbitError> {
    let (status, generation, activated_at): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT status, activation_generation, activated_at
             FROM hub_knowledge_allocator_state WHERE id = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| OrbitError::Store(format!("read knowledge allocator state: {error}")))?;
    let status = match status.as_str() {
        "dormant" => HubKnowledgeAllocatorStatus::Dormant,
        "active" => HubKnowledgeAllocatorStatus::Active,
        other => {
            return Err(OrbitError::Store(format!(
                "invalid knowledge allocator status '{other}'"
            )));
        }
    };
    let activated_at = activated_at
        .as_deref()
        .map(super::parse_timestamp)
        .transpose()
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(HubKnowledgeAllocatorState {
        status,
        activation_generation: u64::try_from(generation).map_err(|error| {
            OrbitError::Store(format!("invalid activation generation: {error}"))
        })?,
        activated_at,
        adr_next_sequence: read_next_sequence(conn, KnowledgeIdKind::Adr)?,
        learning_next_sequence: read_next_sequence(conn, KnowledgeIdKind::Learning)?,
    })
}

fn read_cutover_state(conn: &Connection) -> Result<KnowledgeAuthorityCutoverState, OrbitError> {
    let (status, generation, last_error): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT status, generation, last_error
             FROM hub_knowledge_cutover_state WHERE id = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| OrbitError::Store(format!("read knowledge cutover state: {error}")))?;
    let status = match status.as_str() {
        "pre-activation" => KnowledgeAuthorityCutoverStatus::PreActivation,
        "reconciling" => KnowledgeAuthorityCutoverStatus::Reconciling,
        "active" => KnowledgeAuthorityCutoverStatus::Active,
        "failed-incomplete" => KnowledgeAuthorityCutoverStatus::FailedIncomplete,
        other => {
            return Err(OrbitError::Store(format!(
                "invalid knowledge cutover status '{other}'"
            )));
        }
    };
    Ok(KnowledgeAuthorityCutoverState {
        status,
        generation: u64::try_from(generation).map_err(|error| {
            OrbitError::Store(format!("invalid knowledge cutover generation: {error}"))
        })?,
        last_error,
    })
}

fn read_next_sequence(conn: &Connection, kind: KnowledgeIdKind) -> Result<u64, OrbitError> {
    let raw: i64 = conn
        .query_row(
            "SELECT next_sequence FROM hub_knowledge_sequences WHERE kind = ?1",
            [kind.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    u64::try_from(raw).map_err(|error| OrbitError::Store(format!("invalid sequence: {error}")))
}

fn validate_request_identity(
    request: &HubKnowledgeAllocationRequestV1,
    identity: &HubKnowledgeRequestIdentityV1,
    audit: &AuditEventInsertParams,
) -> Result<(), OrbitError> {
    validate_correlation(&identity.mcp_call_id)?;
    if identity.schema_version != HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION
        || identity.workspace_id != request.workspace_id
        || identity.kind != request.kind
        || identity.model != request.model
    {
        return Err(OrbitError::InvalidInput(
            "hub knowledge allocation request identity does not match the typed request"
                .to_string(),
        ));
    }
    for (label, value) in [
        ("caller_machine_id", identity.caller_machine_id.as_str()),
        ("caller_host_id", identity.caller_host_id.as_str()),
        ("process_machine_id", identity.process_machine_id.as_str()),
        ("process_host_id", identity.process_host_id.as_str()),
        ("origin_session_id", identity.origin_session_id.as_str()),
    ] {
        if value.trim().is_empty() || value.trim() != value {
            return Err(OrbitError::InvalidInput(format!(
                "hub knowledge allocation requires normalized {label}"
            )));
        }
    }
    if identity.effective_capabilities.is_empty() {
        return Err(OrbitError::InvalidInput(
            "hub knowledge allocation requires a non-empty effective capability set".to_string(),
        ));
    }
    if audit.workspace_id.as_deref() != Some(request.workspace_id.as_str())
        || audit.mcp_call_id.as_deref() != Some(identity.mcp_call_id.as_str())
        || audit.caller_machine_id.as_deref() != Some(identity.caller_machine_id.as_str())
        || audit.caller_host_id.as_deref() != Some(identity.caller_host_id.as_str())
        || audit.process_machine_id.as_deref() != Some(identity.process_machine_id.as_str())
        || audit.process_host_id.as_deref() != Some(identity.process_host_id.as_str())
        || audit.transport != Some(identity.transport)
        || audit.effective_capabilities != identity.effective_capabilities
        || audit.origin_session_id.as_deref() != Some(identity.origin_session_id.as_str())
    {
        return Err(OrbitError::InvalidInput(
            "hub knowledge allocation audit provenance does not match the request identity"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_correlation(mcp_call_id: &str) -> Result<(), OrbitError> {
    if mcp_call_id.trim().is_empty() || mcp_call_id.trim() != mcp_call_id {
        return Err(OrbitError::InvalidInput(
            "hub knowledge allocation requires a normalized mcp_call_id".to_string(),
        ));
    }
    Ok(())
}

fn allocation_by_call(
    conn: &Connection,
    mcp_call_id: &str,
) -> Result<Option<(HubKnowledgeAllocationV1, String)>, OrbitError> {
    conn.query_row(
        "SELECT workspace_id, kind, id, sequence, mcp_call_id, allocated_at,
                request_identity_json
         FROM hub_knowledge_allocation_ledger WHERE mcp_call_id = ?1",
        [mcp_call_id],
        |row| Ok((allocation_row(row)?, row.get(6)?)),
    )
    .optional()
    .map_err(|error| OrbitError::Store(error.to_string()))
}

fn allocation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HubKnowledgeAllocationV1> {
    let kind_raw: String = row.get(1)?;
    let kind = match kind_raw.as_str() {
        "adr" => KnowledgeIdKind::Adr,
        "learning" => KnowledgeIdKind::Learning,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let sequence_raw: i64 = row.get(3)?;
    let allocated_at_raw: String = row.get(5)?;
    let allocated_at = DateTime::parse_from_rfc3339(&allocated_at_raw)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .with_timezone(&Utc);
    Ok(HubKnowledgeAllocationV1 {
        schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
        workspace_id: row.get(0)?,
        kind,
        id: row.get(2)?,
        sequence: u32::try_from(sequence_raw)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
        mcp_call_id: row.get(4)?,
        allocated_at,
    })
}

fn u64_to_i64(value: u64, label: &str) -> Result<i64, OrbitError> {
    i64::try_from(value)
        .map_err(|error| OrbitError::Store(format!("{label} is too large: {error}")))
}
