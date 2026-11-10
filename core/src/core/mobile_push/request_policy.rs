use super::*;

// Read-only notification processes use the same private acceptance decisions.
// Unconnected requests stay quiet even when their inbox is enabled.
pub(in crate::core) fn allows(conn: &rusqlite::Connection, owner: &str, peer: &str) -> bool {
    let check =
        || -> anyhow::Result<bool> {
            let key = serde_json::json!(["privateEvent", "requestDecision", peer]);
            let record: Option<String> = conn
                .query_row(
                    "SELECT value FROM app_meta WHERE key=?1",
                    [format!(
                        "iris-chat-sync-record-v1:{owner}:private-state:{key}"
                    )],
                    |row| row.get(0),
                )
                .optional()?;
            let mut reset = false;
            if let Some(record) = record {
                let record: serde_json::Value = serde_json::from_str(&record)?;
                let event: PrivateEvent =
                    serde_json::from_value(record.get("event").cloned().unwrap_or_default())?;
                let (_, target, value) =
                    nostr_identity::parse_private_setting(&event, PublicKey::from_hex(owner)?)?;
                anyhow::ensure!(target == peer, "Request decision target mismatch");
                match value.get("decision").and_then(|decision| decision.as_str()) {
                    Some("accepted") => return Ok(true),
                    Some("rejected") => return Ok(false),
                    Some("none") => reset = true,
                    _ => anyhow::bail!("Invalid request decision"),
                }
            }
            let (network, overmuted, accepted): (bool, bool, bool) = conn.query_row(
                "SELECT accept_social_graph_direct_messages, accept_overmuted_direct_messages,
                EXISTS(SELECT 1 FROM json_each(accepted_owner_pubkeys_json) WHERE value=?1)
                OR EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND is_outgoing=1)
             FROM preferences WHERE id=1",
                [peer],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            if accepted && !reset {
                return Ok(true);
            }
            let bytes: Option<Vec<u8>> = conn.query_row(
            "SELECT social_graph FROM user_discovery_state WHERE id=1 AND owner_pubkey_hex=?1",
            [owner], |row| row.get(0),
        ).optional()?.flatten();
            let Some(bytes) = bytes else { return Ok(false) };
            let graph = nostr_social_graph::SocialGraph::from_binary(owner, &bytes)?;
            let distance = graph.get_follow_distance(peer);
            Ok((distance == 1 && !reset)
                || (network && distance < 1000 && (overmuted || !graph.is_overmuted(peer, 1.0))))
        };
    check().unwrap_or(false)
}

pub(super) fn allows_in_data_dir(data_dir: &str, owner: &str, peer: &str) -> bool {
    open_lookup_connection(data_dir).is_some_and(|conn| allows(&conn, owner, peer))
}
