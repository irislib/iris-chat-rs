use super::*;
use std::collections::BTreeSet;

impl AppCore {
    pub(in crate::core) fn prepare_device_link_roster_repair(
        &mut self,
        token: &str,
        heads: &[Event],
    ) -> anyhow::Result<Event> {
        let pending = self
            .pending_device_link_signer
            .as_ref()
            .filter(|pending| {
                pending.token == token
                    && pending.deadline > Instant::now()
                    && pending.signed.is_none()
            })
            .ok_or_else(|| anyhow::anyhow!("Device approval expired."))?;
        let logged = self
            .logged_in
            .as_ref()
            .filter(|login| {
                login.owner_pubkey == pending.owner
                    && login.authorization_state == LocalAuthorizationState::Authorized
            })
            .ok_or_else(|| anyhow::anyhow!("Device approval cancelled."))?;
        let keys = logged
            .owner_keys
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Primary device required."))?;
        anyhow::ensure!(
            (2..=1024).contains(&heads.len()),
            "Invalid device list repair."
        );
        let template = heads
            .iter()
            .min_by_key(|head| head.id)
            .ok_or_else(|| anyhow::anyhow!("Missing device list."))?;
        let normalized = normalized_roster_subject(template)?;
        let now = unix_now().get();
        for head in heads {
            anyhow::ensure!(
                head.pubkey == pending.owner
                    && is_app_keys_event(head)
                    && head.created_at == template.created_at
                    && head.content.is_empty()
                    && head.created_at.as_secs() <= now.saturating_add(300)
                    && head.verify().is_ok()
                    && normalized_roster_subject(head)? == normalized,
                "Conflicting device lists."
            );
        }
        let baseline = AppKeys::from_event(template)?;
        let known = self
            .app_keys
            .get(&pending.owner.to_hex())
            .ok_or_else(|| anyhow::anyhow!("Device list is unavailable."))?;
        let membership = |roster: &AppKeys| {
            roster
                .get_all_devices()
                .into_iter()
                .map(|device| (device.identity_pubkey, device.created_at))
                .collect::<BTreeMap<_, _>>()
        };
        anyhow::ensure!(
            known.created_at_secs <= template.created_at.as_secs()
                && membership(&known_app_keys_to_ndr(known)) == membership(&baseline)
                && baseline
                    .get_device(&logged.device_keys.public_key())
                    .is_some(),
            "Device list changed."
        );
        let ids = heads.iter().map(|event| event.id).collect::<BTreeSet<_>>();
        anyhow::ensure!(ids.len() > 1, "Device list repair is unnecessary.");
        if let Some((previous, event)) = &pending.repair {
            anyhow::ensure!(*previous == ids, "Device list changed.");
            return Ok(event.clone());
        }
        let created_at = now.max(template.created_at.as_secs().saturating_add(1));
        anyhow::ensure!(
            created_at <= now.saturating_add(300),
            "Device list revision is too new."
        );
        let event = UnsignedEvent::new(
            pending.owner,
            Timestamp::from(created_at),
            template.kind,
            template.tags.clone().to_vec(),
            template.content.clone(),
        )
        .sign_with_keys(keys)?;
        if let Some(pending) = self.pending_device_link_signer.as_mut() {
            pending.repair = Some((ids, event.clone()));
        }
        Ok(event)
    }
}

fn normalized_roster_subject(event: &Event) -> anyhow::Result<Vec<Vec<String>>> {
    let mut tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    let d = tags
        .iter()
        .filter(|tag| tag.first().is_some_and(|key| key == "d"))
        .collect::<Vec<_>>();
    let i = tags
        .iter()
        .filter(|tag| tag.first().is_some_and(|key| key == "i"))
        .collect::<Vec<_>>();
    let [d] = d.as_slice() else {
        anyhow::bail!("Invalid device profile.");
    };
    let [i] = i.as_slice() else {
        anyhow::bail!("Invalid device subject.");
    };
    let [_, subject] = d.as_slice() else {
        anyhow::bail!("Invalid device profile.");
    };
    let [_, indexed, relation] = i.as_slice() else {
        anyhow::bail!("Invalid device subject.");
    };
    anyhow::ensure!(
        uuid::Uuid::parse_str(subject)?.to_string() == *subject
            && indexed == subject
            && relation == "subject",
        "Invalid device subject."
    );
    for tag in &mut tags {
        if tag.first().is_some_and(|key| key == "d" || key == "i") {
            if let Some(subject) = tag.get_mut(1) {
                *subject = "subject".into();
            }
        }
    }
    Ok(tags)
}
