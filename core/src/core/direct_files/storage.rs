use super::*;
use rusqlite::{params, OptionalExtension};

pub(super) fn load(db: &SharedConnection, id: &str) -> Result<Option<Record>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let json: Option<String> = conn
        .query_row(
            "SELECT record_json FROM direct_file_transfers WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    json.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .transpose()
}

pub(super) fn save(db: &SharedConnection, record: &Record) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO direct_file_transfers(id,record_json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET record_json=excluded.record_json",
        params![record.offer.id, serde_json::to_string(record).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(())
}

/// A process/transport restart cannot replay an already accepted capability.
pub(super) fn all(db: &SharedConnection) -> Result<Vec<Record>, String> {
    let records = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT record_json FROM direct_file_transfers")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    records
        .into_iter()
        .map(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
        .collect()
}

pub(super) fn interrupt(db: &SharedConnection, preserve_unregistered: bool) -> Result<(), String> {
    for mut record in all(db)? {
        // No capability has existed yet for a queued file-first offer. It may survive
        // a transport reconfiguration, but never a process restart.
        let unregistered = record.is_sender
            && record.waiting_for_devices
            && record.status == DirectFileTransferStatus::Offered;
        if active(&record.status) && !(preserve_unregistered && unregistered) {
            record.status = DirectFileTransferStatus::Unavailable;
            record.error = Some("Transfer interrupted. Send the files again.".into());
            save(db, &record)?;
        }
    }
    Ok(())
}
