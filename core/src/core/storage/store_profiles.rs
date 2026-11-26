use super::*;

pub(super) fn load_owner_profiles(
    conn: &rusqlite::Connection,
) -> anyhow::Result<BTreeMap<String, OwnerProfileRecord>> {
    let mut stmt = conn.prepare(
        "SELECT owner_pubkey_hex, nickname, name, display_name, picture, about,
                extra_metadata_json, extra_tags_json, updated_at_secs, contact_note, contact_updated_at_ms, contact_memory_json,
                (SELECT value FROM app_meta WHERE key='iris-profile-projection-v1:' || owner_pubkey_hex)
         FROM owner_profiles",
    )?;
    let rows = stmt.query_map([], |row| {
        let owner_pubkey_hex: String = row.get(0)?;
        let extra_metadata_json: String = row.get(6)?;
        let extra_tags_json: String = row.get(7)?;
        let extra_tags: Vec<Vec<String>> =
            serde_json::from_str(&extra_tags_json).unwrap_or_default();
        let record = OwnerProfileRecord {
            source_event_id: row.get(12)?,
            contact_memory: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
            nickname: row.get(1)?,
            contact_note: row.get(9)?,
            contact_updated_at_ms: row.get::<_, i64>(10)? as u64,
            name: row.get(2)?,
            display_name: row.get(3)?,
            picture: row.get(4)?,
            about: row.get(5)?,
            extra_metadata_json,
            extra_tags,
            updated_at_secs: row.get::<_, i64>(8)? as u64,
        };
        Ok((owner_pubkey_hex, record))
    })?;
    let mut profiles = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        profiles.insert(key, value);
    }
    Ok(profiles)
}

pub(super) fn write_owner_profiles_diff(
    tx: &Transaction,
    profiles: &BTreeMap<String, OwnerProfileRecord>,
    diff: &MapDiff,
) -> anyhow::Result<()> {
    let mut delete_meta = tx.prepare_cached("DELETE FROM app_meta WHERE key = ?1")?;
    if diff.full_replace {
        tx.execute("DELETE FROM owner_profiles", [])?;
        tx.execute(
            "DELETE FROM app_meta WHERE key LIKE 'iris-profile-projection-v1:%'",
            [],
        )?;
    } else if !diff.deletes.is_empty() {
        let mut delete_profile =
            tx.prepare_cached("DELETE FROM owner_profiles WHERE owner_pubkey_hex = ?1")?;
        for owner_pubkey_hex in &diff.deletes {
            delete_profile.execute([owner_pubkey_hex])?;
            delete_meta.execute([format!("iris-profile-projection-v1:{owner_pubkey_hex}")])?;
        }
    }
    if diff.upserts.is_empty() {
        return Ok(());
    }
    let mut upsert_meta = tx.prepare_cached(
        "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    let mut upsert_profile = tx.prepare_cached(
        "INSERT INTO owner_profiles
            (owner_pubkey_hex, nickname, name, display_name, picture, about,
             extra_metadata_json, extra_tags_json, updated_at_secs, contact_note, contact_updated_at_ms, contact_memory_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(owner_pubkey_hex) DO UPDATE SET
            nickname = excluded.nickname,
            name = excluded.name,
            display_name = excluded.display_name,
            picture = excluded.picture,
            about = excluded.about,
            extra_metadata_json = excluded.extra_metadata_json,
            extra_tags_json = excluded.extra_tags_json,
            updated_at_secs = excluded.updated_at_secs,
            contact_note = excluded.contact_note,
            contact_updated_at_ms = excluded.contact_updated_at_ms,
            contact_memory_json = excluded.contact_memory_json",
    )?;
    for (owner_pubkey_hex, _) in &diff.upserts {
        let Some(profile) = profiles.get(owner_pubkey_hex) else {
            continue;
        };
        let meta_key = format!("iris-profile-projection-v1:{owner_pubkey_hex}");
        if let Some(id) = &profile.source_event_id {
            upsert_meta.execute(params![meta_key, id])?;
        } else if !diff.full_replace {
            delete_meta.execute([meta_key])?;
        }
        let extra_tags_json =
            serde_json::to_string(&profile.extra_tags).unwrap_or_else(|_| "[]".to_string());
        upsert_profile.execute(params![
            owner_pubkey_hex,
            profile.nickname,
            profile.name,
            profile.display_name,
            profile.picture,
            profile.about,
            profile.extra_metadata_json,
            extra_tags_json,
            profile.updated_at_secs as i64,
            profile.contact_note,
            profile.contact_updated_at_ms as i64,
            serde_json::to_string(&profile.contact_memory)?,
        ])?;
    }
    Ok(())
}
