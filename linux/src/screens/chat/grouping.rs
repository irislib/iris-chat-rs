use super::*;

pub(super) fn cluster_break(
    previous: &ChatMessageSnapshot,
    next: &ChatMessageSnapshot,
    kind: &ChatKind,
) -> bool {
    previous.kind != ChatMessageKind::User
        || next.kind != ChatMessageKind::User
        || previous.call.is_some()
        || next.call.is_some()
        || !previous.reactions.is_empty()
        || previous.is_outgoing != next.is_outgoing
        || next.created_at_secs < previous.created_at_secs
        || next
            .created_at_secs
            .saturating_sub(previous.created_at_secs)
            >= 180
        || !same_day(previous.created_at_secs, next.created_at_secs)
        || (*kind == ChatKind::Group && !next.is_outgoing && author(previous) != author(next))
}

pub(super) fn show_footer(
    message: &ChatMessageSnapshot,
    next: Option<&ChatMessageSnapshot>,
    kind: &ChatKind,
) -> bool {
    next.is_none_or(|next| {
        cluster_break(message, next, kind)
            || message.expires_at_secs.is_some()
            || matches!(
                message.delivery,
                DeliveryState::Pending | DeliveryState::Queued | DeliveryState::Failed
            )
            || message.created_at_secs / 60 != next.created_at_secs / 60
            || message.delivery != next.delivery
    })
}

fn author(message: &ChatMessageSnapshot) -> &str {
    message
        .author_owner_pubkey_hex
        .as_deref()
        .filter(|owner| !owner.is_empty())
        .unwrap_or(&message.author)
}

pub(super) fn local_day(secs: u64) -> Option<String> {
    glib::DateTime::from_unix_local(secs.try_into().ok()?)
        .ok()?
        .format("%Y-%m-%d")
        .ok()
        .map(Into::into)
}

pub(super) fn same_day(left: u64, right: u64) -> bool {
    local_day(left).is_some_and(|day| Some(day) == local_day(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> ChatMessageSnapshot {
        ChatMessageSnapshot {
            edit_history: vec![],
            deleted_for_everyone: false,
            id: "first".into(),
            chat_id: "chat".into(),
            kind: ChatMessageKind::User,
            author: "Same name".into(),
            author_owner_pubkey_hex: Some("person-a".into()),
            author_picture_url: None,
            body: "Hello".into(),
            attachments: vec![],
            reactions: vec![],
            reactors: vec![],
            is_outgoing: false,
            created_at_secs: glib::DateTime::from_local(2026, 10, 3, 12, 0, 0.0)
                .unwrap()
                .to_unix() as u64,
            expires_at_secs: None,
            delivery: DeliveryState::Received,
            recipient_deliveries: vec![],
            delivery_trace: Default::default(),
            source_event_id: None,
            system_notice_owner_pubkey_hex: None,
            direct_transfer: None,
            call: None,
        }
    }

    #[test]
    fn grouping_boundaries_and_stable_identity() {
        let mut first = message();
        let mut next = first.clone();
        next.created_at_secs += 179;
        assert!(!cluster_break(&first, &next, &ChatKind::Group));
        next.created_at_secs += 1;
        assert!(cluster_break(&first, &next, &ChatKind::Group));
        next.created_at_secs = first.created_at_secs - 1;
        assert!(cluster_break(&first, &next, &ChatKind::Group));
        next.created_at_secs = first.created_at_secs + 1;
        next.author = "Renamed".into();
        assert!(!cluster_break(&first, &next, &ChatKind::Group));
        next.author_owner_pubkey_hex = Some("person-b".into());
        assert!(cluster_break(&first, &next, &ChatKind::Group));
        next = first.clone();
        next.is_outgoing = true;
        assert!(cluster_break(&first, &next, &ChatKind::Direct));
        next = first.clone();
        first.reactions.push(MessageReactionSnapshot {
            emoji: "👍".into(),
            count: 1,
            reacted_by_me: false,
        });
        assert!(cluster_break(&first, &next, &ChatKind::Direct));
        assert!(!cluster_break(&next, &first, &ChatKind::Direct));
        first.reactions.clear();
        next.call = Some(iris_chat_core::CallHistorySnapshot {
            call_id: "call".into(),
            direction: "incoming".into(),
            outcome: "missed".into(),
            video: false,
            started_at_secs: first.created_at_secs,
            answered_at_secs: None,
            ended_at_secs: first.created_at_secs,
            duration_secs: 0,
        });
        assert!(cluster_break(&first, &next, &ChatKind::Direct));
        assert!(cluster_break(&next, &first, &ChatKind::Direct));
        next.call = None;
        next.kind = ChatMessageKind::System;
        assert!(cluster_break(&first, &next, &ChatKind::Direct));
        next.kind = ChatMessageKind::User;
        next.created_at_secs = glib::DateTime::from_local(2026, 10, 3, 0, 0, 0.0)
            .unwrap()
            .to_unix() as u64;
        first.created_at_secs = next.created_at_secs - 1;
        assert!(cluster_break(&first, &next, &ChatKind::Direct));
    }

    #[test]
    fn footer_is_independent_from_corners() {
        let mut first = message();
        first.created_at_secs = 120;
        let mut next = first.clone();
        next.created_at_secs += 1;
        assert!(!show_footer(&first, Some(&next), &ChatKind::Direct));
        assert!(show_footer(&first, None, &ChatKind::Direct));
        next.created_at_secs = 180;
        assert!(show_footer(&first, Some(&next), &ChatKind::Direct));
        assert!(!cluster_break(&first, &next, &ChatKind::Direct));
        next.created_at_secs = 121;
        next.delivery = DeliveryState::Seen;
        assert!(show_footer(&first, Some(&next), &ChatKind::Direct));
        for status in [
            DeliveryState::Pending,
            DeliveryState::Queued,
            DeliveryState::Failed,
        ] {
            first.delivery = status.clone();
            next.delivery = status;
            assert!(show_footer(&first, Some(&next), &ChatKind::Direct));
        }
        first.delivery = DeliveryState::Received;
        next.delivery = DeliveryState::Received;
        first.expires_at_secs = Some(1000);
        assert!(show_footer(&first, Some(&next), &ChatKind::Direct));
    }
}
