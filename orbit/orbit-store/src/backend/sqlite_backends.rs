use chrono::{DateTime, Utc};
use orbit_types::{AuditEvent, OrbitError, StoredTool};

use super::contracts::{AuditEventStoreBackend, ToolStoreBackend};
use crate::Store;
use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};

#[derive(Clone)]
pub(crate) struct SqliteToolStoreBackend {
    pub(crate) store: Store,
}

impl ToolStoreBackend for SqliteToolStoreBackend {
    fn list_tools(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.store.list_tools()
    }

    fn get_tool(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.store.get_tool(name)
    }

    fn insert_tool(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.store.with_transaction(|tx| tx.insert_tool(tool))
    }

    fn delete_tool(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| tx.delete_tool(name))
    }

    fn set_tool_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.set_tool_enabled(name, enabled))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteAuditEventStoreBackend {
    pub(crate) store: Store,
}

impl AuditEventStoreBackend for SqliteAuditEventStoreBackend {
    fn insert_audit_event_record(&self, params: &AuditEventInsertParams) -> Result<(), OrbitError> {
        self.store.insert_audit_event_record(params)
    }

    fn list_audit_events(&self, filter: &AuditEventFilter) -> Result<Vec<AuditEvent>, OrbitError> {
        self.store.list_audit_events(filter)
    }

    fn get_audit_event(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.store.get_audit_event(id)
    }

    fn get_audit_event_stats(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.store.get_audit_event_stats(since, tool)
    }

    fn get_audit_event_durations(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.store.get_audit_event_durations(since, tool)
    }

    fn prune_audit_events(&self, older_than: &DateTime<Utc>) -> Result<usize, OrbitError> {
        self.store.prune_audit_events(older_than)
    }
}
