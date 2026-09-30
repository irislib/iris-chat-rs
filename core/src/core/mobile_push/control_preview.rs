use super::*;

const CACHE_PREFIX: &str = "mobile_push_control:";
const CACHE_LIMIT: usize = 256;

// Retain only the short presentation of authenticated controls, never their
// contents or keys. The live runtime may consume the ratchet before the NSE.
#[derive(serde::Serialize, serde::Deserialize)]
struct ControlPreview {
    owner: String,
    sender: String,
    chat_id: String,
    kind: u64,
    body: String,
    typing_until: Option<u64>,
}

pub(super) fn body(kind: u64, content: &str, inner_json: &str) -> String {
    if kind == TYPING_KIND as u64 {
        if let Some(rumor) = parse_runtime_rumor(inner_json) {
            let expiration = message_expiration_from_tags(rumor.tags.iter());
            if expiration.is_some_and(|time| time <= rumor.created_at_secs) {
                return "Stopped typing".to_string();
            }
            if typing_until(&rumor) <= unix_now().get() {
                return "Typing update".to_string();
            }
        }
        return "Typing…".to_string();
    }
    decrypted_mobile_push_body(kind, content)
}

fn typing_until(rumor: &RuntimeRumor) -> u64 {
    message_expiration_from_tags(rumor.tags.iter())
        .unwrap_or(u64::MAX)
        .min(rumor.created_at_secs.saturating_add(10))
}

impl AppCore {
    pub(in crate::core) fn cache_mobile_push_control(
        &mut self,
        outer_event_id: Option<&str>,
        sender: PublicKey,
        chat_id: &str,
        rumor: &RuntimeRumor,
    ) {
        let Some(owner) = self.logged_in.as_ref().map(|account| account.owner_pubkey) else {
            return;
        };
        let Some(outer_event_id) = outer_event_id else {
            return;
        };
        if sender == owner
            || !matches!(
                rumor.kind,
                RECEIPT_KIND | TYPING_KIND | CHAT_SETTINGS_KIND | REACTION_KIND
            )
        {
            return;
        }
        let preview = ControlPreview {
            owner: owner.to_hex(),
            sender: sender.to_hex(),
            chat_id: chat_id.to_string(),
            kind: rumor.kind as u64,
            body: if rumor.kind == TYPING_KIND {
                if message_expiration_from_tags(rumor.tags.iter())
                    .is_some_and(|time| time <= rumor.created_at_secs)
                {
                    "Stopped typing".to_string()
                } else {
                    "Typing…".to_string()
                }
            } else {
                decrypted_mobile_push_body(rumor.kind as u64, &rumor.content)
            },
            typing_until: (rumor.kind == TYPING_KIND).then(|| typing_until(rumor)),
        };
        let save = || -> anyhow::Result<()> {
            let shared = self.app_store.shared();
            let mut conn = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
                rusqlite::params![
                    format!("{CACHE_PREFIX}{outer_event_id}"),
                    serde_json::to_string(&preview)?
                ],
            )?;
            tx.execute("DELETE FROM app_meta WHERE key IN (SELECT key FROM app_meta WHERE key LIKE 'mobile_push_control:%' ORDER BY rowid DESC LIMIT -1 OFFSET ?1)", [CACHE_LIMIT])?;
            tx.commit()?;
            Ok(())
        };
        if let Err(error) = save() {
            self.push_debug_log("mobile_push.control_preview.error", error.to_string());
        }
    }

    pub(in crate::core) fn cache_group_mobile_push_control(
        &mut self,
        outer_id: &str,
        event: &GroupIncomingEvent,
    ) {
        let GroupIncomingEvent::Message(message) = event else {
            return;
        };
        let Ok(sender) = PublicKey::from_slice(&message.sender_owner.to_bytes()) else {
            return;
        };
        let Some(rumor) = parse_runtime_rumor(&String::from_utf8_lossy(&message.body)) else {
            return;
        };
        let device = message
            .sender_device
            .and_then(|device| PublicKey::from_slice(&device.to_bytes()).ok());
        if self.runtime_rumor_pubkey_matches_authenticated_sender(sender, device, rumor.pubkey) {
            self.cache_mobile_push_control(
                Some(outer_id),
                sender,
                &group_chat_id(&message.group_id),
                &rumor,
            );
        }
    }
}

pub(super) fn lookup(
    conn: &rusqlite::Connection,
    data_dir: &str,
    outer_id: &str,
    owner_pubkey_hex: &str,
) -> Option<MobilePushNotificationResolution> {
    let json: String = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [format!("{CACHE_PREFIX}{outer_id}")],
            |row| row.get(0),
        )
        .ok()?;
    let preview: ControlPreview = serde_json::from_str(&json).ok()?;
    if !preview.owner.eq_ignore_ascii_case(owner_pubkey_hex) {
        return Some(suppressed_resolution());
    }
    super::super::storage::validate_account_storage(conn, &preview.owner).ok()?;
    if is_chat_muted_in(conn, &preview.chat_id) {
        return Some(suppressed_resolution());
    }
    let sender = PublicKey::from_hex(&preview.sender).ok()?;
    let sender_name = lookup_owner_display_name(conn, &sender)
        .or_else(|| lookup_direct_thread_sender_name(data_dir, &sender))
        .unwrap_or_else(|| "Iris Chat".to_string());
    let group_id = preview.chat_id.strip_prefix(GROUP_CHAT_PREFIX);
    let group_title = group_id.and_then(|id| lookup_group_name_in(conn, id));
    let body = if preview.body == "Typing…"
        && preview
            .typing_until
            .is_some_and(|until| until <= unix_now().get())
    {
        "Typing update".to_string()
    } else {
        preview.body
    };
    let (title, body) = group_notification_title_and_body(group_title, &sender_name, body);
    Some(MobilePushNotificationResolution {
        should_show: false,
        payload_json: serde_json::json!({"title": title, "body": body,
            "inner_kind": preview.kind.to_string(), "sender_pubkey": preview.sender,
            "chat_id": preview.chat_id, "group_id": group_id})
        .to_string(),
        title,
        body,
    })
}
