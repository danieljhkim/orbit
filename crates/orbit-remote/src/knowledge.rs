//! Explicit activation and checkoutless hub knowledge-allocation service.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    AuditEventStatus, HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, KnowledgeIdKind, OrbitError,
    ToolSessionContext, WorkspaceRegistry, WorkspaceStatus, audit_execution_id,
};
use orbit_core::command::tool::{
    ToolEntryPoint, audit_role_label_for_entry_point, trusted_mcp_audit_context,
};
use orbit_store::AuditEventInsertParams;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use crate::persistence::{
    HubKnowledgeAllocatorState, HubKnowledgeRequestIdentityV1, KnowledgeWorkspaceInventory,
    LegacyKnowledgeId,
};
use crate::{RemoteStore, workspace_registry};

#[derive(Clone)]
pub struct HubKnowledgeSequenceService {
    store: RemoteStore,
    registered_workspace_ids: BTreeSet<String>,
    global_root: Option<PathBuf>,
}

impl HubKnowledgeSequenceService {
    pub fn at(global_root: &Path) -> Result<Self, OrbitError> {
        let identity = crate::host_registry::require_local_hub_identity(global_root)?;
        let store = crate::remote_store_at(global_root)?;
        require_configured_hub_identity(&store, &identity.machine_id)?;
        let registry = workspace_registry::load_registry_from(
            &workspace_registry::registry_path_for(global_root),
        )?;
        let registered_workspace_ids = registered_workspace_ids(&registry);
        Ok(Self {
            store,
            registered_workspace_ids,
            global_root: Some(global_root.to_path_buf()),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        store: RemoteStore,
        registered_workspace_ids: BTreeSet<String>,
    ) -> Self {
        Self {
            store,
            registered_workspace_ids,
            global_root: None,
        }
    }

    // F1 deliberately installs a dormant substrate; F3 will call this during cutover.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn activate(
        &self,
        inventories: Vec<KnowledgeWorkspaceInventory>,
    ) -> Result<HubKnowledgeAllocatorState, OrbitError> {
        self.require_hub_mode()?;
        self.require_exact_inventory_coverage(
            &inventories,
            &self.current_registered_workspace_ids()?,
        )?;
        self.store.activate_knowledge_allocator(inventories)
    }

    // F1 deliberately installs a dormant substrate; F3 will call this for late workspaces.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn reconcile_workspace(
        &self,
        inventory: KnowledgeWorkspaceInventory,
    ) -> Result<HubKnowledgeAllocatorState, OrbitError> {
        self.require_hub_mode()?;
        if !self
            .current_registered_workspace_ids()?
            .contains(&inventory.workspace_id)
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot reconcile unregistered workspace '{}'",
                inventory.workspace_id
            )));
        }
        self.store.reconcile_knowledge_workspace(inventory)
    }

    pub fn allocate(
        &self,
        request: &HubKnowledgeAllocationRequestV1,
        context: &ToolSessionContext,
    ) -> Result<HubKnowledgeAllocationV1, OrbitError> {
        self.require_hub_mode()?;
        request.validate()?;
        if context.effective_capabilities.len() != 1
            || (!context
                .effective_capabilities
                .contains(&orbit_common::types::McpCapability::Agent)
                && !context
                    .effective_capabilities
                    .contains(&orbit_common::types::McpCapability::Operator))
        {
            return Err(OrbitError::InvalidInput(
                "private hub knowledge allocation requires exactly one effective capability: agent or operator"
                    .to_string(),
            ));
        }
        if !self
            .current_active_workspace_ids()?
            .contains(&request.workspace_id)
        {
            return Err(OrbitError::InvalidInput(format!(
                "cannot allocate knowledge for unregistered or inactive workspace '{}'",
                request.workspace_id
            )));
        }
        let identity = request_identity(request, context)?;
        let audit = allocation_audit(request, context, &identity)?;
        self.store
            .allocate_hub_knowledge_id(request, &identity, &audit)
    }

    pub fn allocation_by_call(
        &self,
        mcp_call_id: &str,
    ) -> Result<Option<HubKnowledgeAllocationV1>, OrbitError> {
        self.store.hub_knowledge_allocation_by_call(mcp_call_id)
    }

    pub fn allocation_by_id(
        &self,
        workspace_id: &str,
        kind: KnowledgeIdKind,
        id: &str,
    ) -> Result<Option<HubKnowledgeAllocationV1>, OrbitError> {
        self.store
            .hub_knowledge_allocation_by_id(workspace_id, kind, id)
    }

    fn require_exact_inventory_coverage(
        &self,
        inventories: &[KnowledgeWorkspaceInventory],
        registered_workspace_ids: &BTreeSet<String>,
    ) -> Result<(), OrbitError> {
        let supplied = inventories
            .iter()
            .map(|inventory| inventory.workspace_id.clone())
            .collect::<BTreeSet<_>>();
        let missing = registered_workspace_ids
            .difference(&supplied)
            .cloned()
            .collect::<Vec<_>>();
        let extra = supplied
            .difference(registered_workspace_ids)
            .cloned()
            .collect::<Vec<_>>();
        if !inventories.is_empty()
            && missing.is_empty()
            && extra.is_empty()
            && supplied.len() == inventories.len()
        {
            return Ok(());
        }
        Err(OrbitError::Migration(format!(
            "knowledge activation inventory must be nonempty and cover the registered-workspace set exactly; missing [{}], extra [{}], repeated inputs {}",
            missing.join(", "),
            extra.join(", "),
            inventories.len().saturating_sub(supplied.len())
        )))
    }

    fn require_hub_mode(&self) -> Result<(), OrbitError> {
        if let Some(global_root) = &self.global_root {
            let identity = crate::host_registry::require_local_hub_identity(global_root)?;
            require_configured_hub_identity(&self.store, &identity.machine_id)?;
        }
        Ok(())
    }

    fn current_registered_workspace_ids(&self) -> Result<BTreeSet<String>, OrbitError> {
        if let Some(global_root) = &self.global_root {
            let registry = workspace_registry::load_registry_from(
                &workspace_registry::registry_path_for(global_root),
            )?;
            return Ok(registered_workspace_ids(&registry));
        }
        Ok(self.registered_workspace_ids.clone())
    }

    fn current_active_workspace_ids(&self) -> Result<BTreeSet<String>, OrbitError> {
        if let Some(global_root) = &self.global_root {
            let registry = workspace_registry::load_registry_from(
                &workspace_registry::registry_path_for(global_root),
            )?;
            return Ok(active_workspace_ids(&registry));
        }
        Ok(self.registered_workspace_ids.clone())
    }
}

pub fn scan_registered_knowledge_inventories(
    global_root: &Path,
) -> Result<Vec<KnowledgeWorkspaceInventory>, OrbitError> {
    let registry = workspace_registry::load_registry_from(&workspace_registry::registry_path_for(
        global_root,
    ))?;
    let mut inventories = BTreeMap::new();
    for workspace in &registry.workspaces {
        let checkout = registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace.id)
            .ok_or_else(|| {
                OrbitError::Migration(format!(
                    "registered workspace '{}' has no hub-local checkout to scan; supply a validated owner inventory before activation",
                    workspace.id
                ))
            })?;
        let mut ids = BTreeMap::new();
        let primary_metadata = fs::symlink_metadata(&checkout.orbit_dir).map_err(|error| {
            OrbitError::Migration(format!(
                "registered workspace '{}' has unresolved migration source '{}': its primary hub-local orbit_dir is unavailable because the checkout binding is stale: {error}",
                workspace.id,
                checkout.orbit_dir.display()
            ))
        })?;
        if !primary_metadata.file_type().is_dir() {
            return Err(OrbitError::Migration(format!(
                "registered workspace '{}' has unresolved migration source '{}': its primary hub-local orbit_dir is not a real directory because the checkout binding is stale",
                workspace.id,
                checkout.orbit_dir.display()
            )));
        }
        let mut orbit_roots = BTreeSet::from([checkout.orbit_dir.clone()]);
        for override_root in &checkout.path_overrides {
            orbit_roots.insert(override_root.join(".orbit"));
        }
        for orbit_root in orbit_roots {
            scan_adr_files(&orbit_root, &mut ids)?;
            scan_learning_files(&orbit_root, &mut ids)?;
            scan_allocation_rows(&orbit_root, &mut ids)?;
        }
        inventories.insert(
            workspace.id.clone(),
            KnowledgeWorkspaceInventory {
                workspace_id: workspace.id.clone(),
                ids: materialize_ids(ids),
            },
        );
    }

    inventories
        .into_values()
        .map(KnowledgeWorkspaceInventory::validated)
        .collect()
}

fn registered_workspace_ids(registry: &WorkspaceRegistry) -> BTreeSet<String> {
    registry
        .workspaces
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect()
}

fn active_workspace_ids(registry: &WorkspaceRegistry) -> BTreeSet<String> {
    registry
        .workspaces
        .iter()
        .filter(|workspace| workspace.status == WorkspaceStatus::Active)
        .map(|workspace| workspace.id.clone())
        .collect()
}

fn require_configured_hub_identity(
    store: &RemoteStore,
    local_machine_id: &str,
) -> Result<(), OrbitError> {
    match store.hub_machine_id()? {
        Some(configured) if configured == local_machine_id => Ok(()),
        Some(configured) => Err(OrbitError::InvalidInput(format!(
            "refusing hub knowledge allocation through a shadow coordination store: local hub machine_id '{local_machine_id}' does not match configured hub_machine_id '{configured}'"
        ))),
        None => Err(OrbitError::InvalidInput(
            "the global coordination store has no configured hub_machine_id; register this hub before activating hub knowledge allocation"
                .to_string(),
        )),
    }
}

type InventoryMap = BTreeMap<(KnowledgeIdKind, String), BTreeSet<String>>;

fn add_evidence(map: &mut InventoryMap, kind: KnowledgeIdKind, id: String, evidence: String) {
    map.entry((kind, id)).or_default().insert(evidence);
}

fn materialize_ids(map: InventoryMap) -> Vec<LegacyKnowledgeId> {
    map.into_iter()
        .map(|((kind, id), evidence)| LegacyKnowledgeId { kind, id, evidence })
        .collect()
}

fn scan_adr_files(orbit_root: &Path, ids: &mut InventoryMap) -> Result<(), OrbitError> {
    for state in ["proposed", "accepted", "superseded", "deleted"] {
        let root = orbit_root.join("adrs").join(state);
        for directory in child_directories_if_present(&root)? {
            let directory_id = file_name_utf8(&directory)?;
            let yaml = directory.join("adr.yaml");
            if !yaml.is_file() {
                continue;
            }
            let document_id = yaml_id(&yaml)?;
            if document_id != directory_id {
                return Err(OrbitError::Migration(format!(
                    "ADR source '{}' contains id '{}' but directory is '{}'",
                    yaml.display(),
                    document_id,
                    directory_id
                )));
            }
            validate_source_id(KnowledgeIdKind::Adr, &document_id, &yaml)?;
            add_evidence(
                ids,
                KnowledgeIdKind::Adr,
                document_id,
                format!("adr-file:{state}"),
            );
        }
    }
    Ok(())
}

fn scan_learning_files(orbit_root: &Path, ids: &mut InventoryMap) -> Result<(), OrbitError> {
    let root = orbit_root.join("learnings");
    for directory in child_directories_if_present(&root)? {
        let directory_id = file_name_utf8(&directory)?;
        let yaml = directory.join("learning.yaml");
        if !yaml.is_file() {
            continue;
        }
        let value = read_yaml(&yaml)?;
        let document_id = yaml_string(&value, "id", &yaml)?;
        if document_id != directory_id {
            return Err(OrbitError::Migration(format!(
                "learning source '{}' contains id '{}' but directory is '{}'",
                yaml.display(),
                document_id,
                directory_id
            )));
        }
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active");
        if !matches!(status, "active" | "superseded") {
            return Err(OrbitError::Migration(format!(
                "learning source '{}' has invalid lifecycle status '{}'",
                yaml.display(),
                status
            )));
        }
        validate_source_id(KnowledgeIdKind::Learning, &document_id, &yaml)?;
        add_evidence(
            ids,
            KnowledgeIdKind::Learning,
            document_id,
            format!("learning-file:{status}"),
        );
    }
    Ok(())
}

fn scan_allocation_rows(orbit_root: &Path, ids: &mut InventoryMap) -> Result<(), OrbitError> {
    let database = orbit_root.join("state").join("semantic.db");
    let Some(conn) = open_read_only_if_present(&database)? else {
        return Ok(());
    };
    if !table_exists(&conn, "id_allocations").map_err(|error| {
        OrbitError::Migration(format!("inspect {}: {error}", database.display()))
    })? {
        return Ok(());
    }
    let mut statement = conn
        .prepare("SELECT kind, id, status FROM id_allocations ORDER BY kind, id")
        .map_err(|error| {
            OrbitError::Migration(format!("inspect {}: {error}", database.display()))
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| OrbitError::Migration(format!("read {}: {error}", database.display())))?;
    for row in rows {
        let (kind, id, status) = row.map_err(|error| OrbitError::Migration(error.to_string()))?;
        let kind = match kind.as_str() {
            "adr" => KnowledgeIdKind::Adr,
            "learning" => KnowledgeIdKind::Learning,
            _ => {
                return Err(OrbitError::Migration(format!(
                    "{} contains unknown id_allocations kind '{}' for '{}'",
                    database.display(),
                    kind,
                    id
                )));
            }
        };
        if !matches!(status.as_str(), "reserved" | "merged" | "abandoned") {
            return Err(OrbitError::Migration(format!(
                "{} contains unknown id_allocations status '{}' for '{}'",
                database.display(),
                status,
                id
            )));
        }
        validate_source_id(kind, &id, &database)?;
        add_evidence(ids, kind, id, format!("allocation:{status}"));
    }
    Ok(())
}

fn request_identity(
    request: &HubKnowledgeAllocationRequestV1,
    context: &ToolSessionContext,
) -> Result<HubKnowledgeRequestIdentityV1, OrbitError> {
    let required = |label: &str, value: &Option<String>| {
        value
            .as_deref()
            .filter(|value| !value.trim().is_empty() && value.trim() == *value)
            .map(str::to_string)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "private hub knowledge allocation requires trusted {label}"
                ))
            })
    };
    if context.workspace_id.as_deref() != Some(request.workspace_id.as_str()) {
        return Err(OrbitError::InvalidInput(
            "private hub knowledge allocation workspace does not match trusted session context"
                .to_string(),
        ));
    }
    let transport = context.transport.ok_or_else(|| {
        OrbitError::InvalidInput(
            "private hub knowledge allocation requires trusted transport".to_string(),
        )
    })?;
    if context.effective_capabilities.is_empty() {
        return Err(OrbitError::InvalidInput(
            "private hub knowledge allocation requires trusted effective capabilities".to_string(),
        ));
    }
    Ok(HubKnowledgeRequestIdentityV1 {
        schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
        workspace_id: request.workspace_id.clone(),
        kind: request.kind,
        model: request.model.clone(),
        caller_machine_id: required("caller_machine_id", &context.caller_machine_id)?,
        caller_host_id: required("caller_host_id", &context.caller_host_id)?,
        process_machine_id: required("process_machine_id", &context.process_machine_id)?,
        process_host_id: required("process_host_id", &context.process_host_id)?,
        transport,
        effective_capabilities: context.effective_capabilities.clone(),
        origin_session_id: required("origin_session_id", &context.origin_session_id)?,
        mcp_call_id: required("mcp_call_id", &context.mcp_call_id)?,
        leased_run: context.leased_run.clone(),
    })
}

fn allocation_audit(
    request: &HubKnowledgeAllocationRequestV1,
    context: &ToolSessionContext,
    identity: &HubKnowledgeRequestIdentityV1,
) -> Result<AuditEventInsertParams, OrbitError> {
    let (correlation, correlation_error) = trusted_mcp_audit_context(context);
    if let Some(error) = correlation_error {
        return Err(error);
    }
    let arguments_json = serde_json::to_string(&json!({
        "schema_version": request.schema_version,
        "kind": request.kind,
        "model": request.model,
    }))
    .map_err(|error| OrbitError::Store(format!("serialize allocation audit: {error}")))?;
    Ok(AuditEventInsertParams {
        execution_id: audit_execution_id("audit-hub-knowledge-allocation"),
        command: "id".to_string(),
        subcommand: Some("allocate".to_string()),
        tool_name: Some(HUB_KNOWLEDGE_ALLOCATION_METHOD_V1.to_string()),
        target_type: Some("id_allocation".to_string()),
        target_id: None,
        role: audit_role_label_for_entry_point(
            &Value::Null,
            None,
            request.model.as_deref(),
            ToolEntryPoint::Mcp,
        ),
        status: AuditEventStatus::Success,
        exit_code: 0,
        duration_ms: 0,
        working_directory: ".".to_string(),
        arguments_json: Some(arguments_json),
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: None,
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        workspace_id: Some(request.workspace_id.clone()),
        caller_machine_id: Some(identity.caller_machine_id.clone()),
        caller_host_id: Some(identity.caller_host_id.clone()),
        process_machine_id: Some(identity.process_machine_id.clone()),
        process_host_id: Some(identity.process_host_id.clone()),
        transport: Some(identity.transport),
        effective_capabilities: identity.effective_capabilities.clone(),
        origin_session_id: Some(identity.origin_session_id.clone()),
        mcp_call_id: Some(identity.mcp_call_id.clone()),
        lease_id: identity.leased_run.as_ref().map(|run| run.lease_id.clone()),
        task_id: correlation.task_id,
        job_run_id: correlation.job_run_id,
        activity_id: correlation.activity_id,
        step_index: correlation.step_index,
    })
}

fn open_read_only_if_present(path: &Path) -> Result<Option<Connection>, OrbitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(OrbitError::Migration(format!(
                "knowledge allocation source '{}' exists but is not a regular file",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(OrbitError::Migration(format!(
                "inspect knowledge allocation source '{}': {error}",
                path.display()
            )));
        }
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map(Some)
    .map_err(|error| OrbitError::Migration(format!("open {} read-only: {error}", path.display())))
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, OrbitError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|error| OrbitError::Migration(error.to_string()))
}

fn child_directories_if_present(root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut directories = fs::read_dir(root)
        .map_err(|error| OrbitError::Migration(format!("read {}: {error}", root.display())))?
        .map(|entry| {
            entry
                .map_err(|error| OrbitError::Migration(error.to_string()))
                .and_then(|entry| {
                    entry
                        .file_type()
                        .map_err(|error| OrbitError::Migration(error.to_string()))
                        .map(|kind| (entry.path(), kind))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    directories.retain(|(_, kind)| kind.is_dir());
    let mut paths = directories
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn file_name_utf8(path: &Path) -> Result<String, OrbitError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            OrbitError::Migration(format!("non-UTF-8 knowledge path '{}'", path.display()))
        })
}

fn read_yaml(path: &Path) -> Result<Value, OrbitError> {
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Migration(format!("read {}: {error}", path.display())))?;
    serde_yaml::from_str(&raw)
        .map_err(|error| OrbitError::Migration(format!("parse {}: {error}", path.display())))
}

fn yaml_id(path: &Path) -> Result<String, OrbitError> {
    let value = read_yaml(path)?;
    yaml_string(&value, "id", path)
}

fn yaml_string(value: &Value, key: &str, path: &Path) -> Result<String, OrbitError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            OrbitError::Migration(format!("{}: missing string field '{key}'", path.display()))
        })
}

fn validate_source_id(kind: KnowledgeIdKind, id: &str, source: &Path) -> Result<(), OrbitError> {
    if kind.parse_id(id).is_none() {
        return Err(OrbitError::Migration(format!(
            "{} contains invalid {} id '{}'",
            source.display(),
            kind.as_str(),
            id
        )));
    }
    Ok(())
}
