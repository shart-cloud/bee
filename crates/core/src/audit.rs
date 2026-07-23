//! Audit event types (FR-007). `AuditEvent` is the JSON-serializable user-space view of a kernel
//! [`bee_common::AuditRecord`], matching `contracts/audit-event.schema.json`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use bee_common::{AuditRecord, Decision, Op};

/// A structured record of a denied (or would-be-denied) operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// ISO-8601 / RFC-3339 wall-clock time.
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub cgroup_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tgid: Option<u32>,
    pub op: String,
    pub decision: String,
    pub errno: i32,
    pub target: String,
}

fn op_str(op: u8) -> &'static str {
    match op {
        x if x == Op::FileOpen as u8 => "file_open",
        x if x == Op::Exec as u8 => "exec",
        x if x == Op::Connect as u8 => "connect",
        x if x == Op::Exfil as u8 => "exfil",
        _ => "unknown",
    }
}

fn decision_str(d: u8) -> &'static str {
    match d {
        x if x == Decision::Denied as u8 => "denied",
        x if x == Decision::Observed as u8 => "observed",
        _ => "unknown",
    }
}

impl AuditEvent {
    /// Build an event from a kernel record. `ts` is the wall-clock time the user-space consumer maps
    /// the record's monotonic `ts_ns` to (the monotonic→wall conversion is a loader concern).
    pub fn from_record(rec: &AuditRecord, scope_id: Option<String>, ts: OffsetDateTime) -> Self {
        let len = (rec.target_len as usize).min(rec.target.len());
        let target = String::from_utf8_lossy(&rec.target[..len]).into_owned();
        AuditEvent {
            ts: ts
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
            scope_id,
            cgroup_id: rec.cgroup_id,
            pid: Some(rec.pid),
            tgid: Some(rec.tgid),
            op: op_str(rec.op).to_string(),
            decision: decision_str(rec.decision).to_string(),
            errno: rec.errno,
            target,
        }
    }

    /// Serialize to a single JSON line.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("AuditEvent is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> AuditRecord {
        let mut target = [0u8; bee_common::TARGET_MAX];
        let s = b"/home/jg/.ssh/id_rsa";
        target[..s.len()].copy_from_slice(s);
        AuditRecord {
            ts_ns: 0,
            cgroup_id: 74213,
            pid: 40122,
            tgid: 40122,
            op: Op::FileOpen as u8,
            decision: Decision::Denied as u8,
            _pad: [0; 2],
            errno: -13,
            target_len: s.len() as u16,
            _pad2: [0; 6],
            target,
        }
    }

    #[test]
    fn maps_record_to_json() {
        let ev = AuditEvent::from_record(
            &rec(),
            Some("cargo-test-1a2b".into()),
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(ev.op, "file_open");
        assert_eq!(ev.decision, "denied");
        assert_eq!(ev.errno, -13);
        assert_eq!(ev.target, "/home/jg/.ssh/id_rsa");
        assert_eq!(ev.cgroup_id, 74213);
        let json = ev.to_json();
        assert!(json.contains("\"op\":\"file_open\""));
        assert!(json.contains("\"target\":\"/home/jg/.ssh/id_rsa\""));
    }
}
