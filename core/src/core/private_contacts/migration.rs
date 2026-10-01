use super::*;

impl AppCore {
    pub(in crate::core) fn retire_legacy_private_contact_intents(
        &mut self,
        engine: &mut ProtocolEngine,
        owner: PublicKey,
    ) -> anyhow::Result<()> {
        // Commit the complete merged registers and V2 outbox before removing V1
        // intents. A crash between these writes retries retirement on next start.
        self.restore_private_contact_state(&owner.to_hex())?;
        engine.retire_pending_local_sibling_events(|conversation, event| {
            conversation == owner && obsolete_private_contact_control(event, owner)
        })?;
        Ok(())
    }
}

fn obsolete_private_contact_control(event: &UnsignedEvent, owner: PublicKey) -> bool {
    if event.kind.as_u16() != 10451 || event.pubkey != owner {
        return false;
    }
    let Ok(serde_json::Value::Object(value)) = serde_json::from_str(&event.content) else {
        return false;
    };
    if value.len() != 3
        || value.get("type").and_then(serde_json::Value::as_str) != Some("private-contact-sync")
        || value.get("v").and_then(serde_json::Value::as_u64) != Some(1)
    {
        return false;
    }
    if value.get("request").and_then(serde_json::Value::as_bool) == Some(true) {
        return true;
    }
    let Some(document) = value.get("document").and_then(serde_json::Value::as_object) else {
        return false;
    };
    if document.len() != 6
        || document.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        || document.get("owner").and_then(serde_json::Value::as_str)
            != Some(owner.to_hex().as_str())
        || !["writer", "record_id"].iter().all(|key| {
            document
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        })
    {
        return false;
    }
    let mut migrated = serde_json::Map::new();
    for key in ["owner", "contact", "fields"] {
        let Some(field) = document.get(key) else {
            return false;
        };
        migrated.insert(key.into(), field.clone());
    }
    migrated.insert("version".into(), serde_json::json!(2));
    serde_json::from_value::<PrivateContactDocumentV2>(migrated.into())
        .is_ok_and(|document| build_private_contact_control_v2(&document).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_matches_only_original_own_account_v1_controls() {
        let owner = Keys::generate().public_key();
        let document = serde_json::json!({
            "version":1, "owner":owner.to_hex(), "contact":Keys::generate().public_key().to_hex(),
            "writer":"a".repeat(32), "record_id":"b".repeat(32),
            "fields":{"favorite":{"counter":1,"writer":"a".repeat(32),"value":true}}
        });
        let control = serde_json::json!({"type":"private-contact-sync","v":1,"document":document});
        let event = |body: serde_json::Value| {
            EventBuilder::new(Kind::from(10451), body.to_string()).build(owner)
        };
        assert!(obsolete_private_contact_control(
            &event(control.clone()),
            owner
        ));
        let mut foreign = control.clone();
        foreign["document"]["owner"] = serde_json::json!(Keys::generate().public_key().to_hex());
        let mut mixed = control.clone();
        mixed["request"] = serde_json::json!(true);
        let mut malformed = control;
        malformed["document"]["fields"]["favorite"]["value"] = serde_json::json!("invalid");
        for body in [
            foreign,
            mixed,
            malformed,
            serde_json::json!({"type":"other","v":1,"request":true}),
        ] {
            assert!(!obsolete_private_contact_control(&event(body), owner));
        }
    }
}
