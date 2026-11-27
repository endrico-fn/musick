use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::storage::playlists::now_i64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxOp {
    pub id: i64,
    pub account_key: String,
    pub op_kind: String,
    pub entity_id: String,
    pub payload_json: String,
    pub created_at: i64,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

pub fn enqueue_outbox(
    conn: &Connection,
    account_key: &str,
    op_kind: &str,
    entity_id: &str,
    payload_json: &str,
) -> CoreResult<i64> {
    let now = now_i64();
    conn.execute(
        "INSERT INTO sync_outbox (account_key, op_kind, entity_id, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![account_key, op_kind, entity_id, payload_json, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_outbox(conn: &Connection, account_key: &str) -> CoreResult<Vec<OutboxOp>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_key, op_kind, entity_id, payload_json, created_at, retry_count, last_error
         FROM sync_outbox
         WHERE account_key = ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([account_key], |row| {
        let retry: i64 = row.get(6)?;
        Ok(OutboxOp {
            id: row.get(0)?,
            account_key: row.get(1)?,
            op_kind: row.get(2)?,
            entity_id: row.get(3)?,
            payload_json: row.get(4)?,
            created_at: row.get(5)?,
            retry_count: u32::try_from(retry.max(0)).unwrap_or(0),
            last_error: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn has_pending_outbox_for_entity(
    conn: &Connection,
    account_key: &str,
    entity_id: &str,
) -> CoreResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE account_key = ?1 AND entity_id = ?2",
        params![account_key, entity_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn complete_outbox(conn: &Connection, op_id: i64) -> CoreResult<()> {
    conn.execute("DELETE FROM sync_outbox WHERE id = ?1", [op_id])?;
    Ok(())
}

pub fn fail_outbox(conn: &Connection, op_id: i64, error_msg: &str) -> CoreResult<()> {
    conn.execute(
        "UPDATE sync_outbox SET retry_count = retry_count + 1, last_error = ?1 WHERE id = ?2",
        params![error_msg, op_id],
    )?;
    Ok(())
}

pub fn count_outbox(conn: &Connection, account_key: &str) -> CoreResult<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE account_key = ?1",
        [account_key],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count.max(0)).unwrap_or(0))
}
