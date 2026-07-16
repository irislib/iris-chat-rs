use super::*;

impl AppCore {
    pub(super) fn reply_device_sync_snapshot(
        &mut self,
        source_pubkey_hex: &str,
        requested_roster_at: u64,
        page: Option<DeviceSyncPage>,
    ) {
        let Some(local_roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let cutoff = local_roster_at.max(requested_roster_at);
        let (mut packets, next) = match page.unwrap_or(DeviceSyncPage::Metadata { offset: 0 }) {
            DeviceSyncPage::Metadata { offset } => {
                (metadata_page_packets(self, cutoff, offset), None)
            }
            DeviceSyncPage::Messages { after } => {
                let (messages, next) = collect_device_sync_messages(
                    self,
                    cutoff,
                    after.as_ref(),
                    DEVICE_SYNC_PAGE_MESSAGES,
                );
                let snapshot = DeviceSyncSnapshot {
                    roster_at: cutoff,
                    messages,
                    ..DeviceSyncSnapshot::default()
                };
                (
                    encode_device_sync_chunks(snapshot),
                    next.map(|after| DeviceSyncPage::Messages { after: Some(after) }),
                )
            }
        };
        if let Some(next) = next {
            let Ok(page_end) = serde_json::to_vec(&DeviceSyncPacket::PageEnd {
                v: DEVICE_SYNC_VERSION,
                roster_at: cutoff,
                next,
            }) else {
                return;
            };
            packets.push(page_end);
        }
        let Some(tcp) = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.tcp.clone())
        else {
            return;
        };
        let Some(peer) = fips_peer_from_hex(source_pubkey_hex) else {
            return;
        };
        let _ = tcp.send_batch(peer, packets);
    }

    pub(super) fn request_device_sync_snapshot(
        &mut self,
        source_pubkey_hex: &str,
        page: Option<DeviceSyncPage>,
    ) {
        let Some(roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let control_rank = page_rank(page.as_ref());
        let Ok(packet) = serde_json::to_vec(&DeviceSyncPacket::Request {
            v: DEVICE_SYNC_VERSION,
            roster_at,
            page,
        }) else {
            return;
        };
        let Some((tcp, peer)) = self.device_sync.as_ref().and_then(|runtime| {
            runtime
                .tcp
                .clone()
                .zip(fips_peer_from_hex(source_pubkey_hex))
        }) else {
            return;
        };
        let _ = tcp.send_control(peer, packet, control_rank);
    }

    #[cfg(test)]
    pub(crate) fn device_sync_message_page_for_test(
        &self,
        roster_at: u64,
        after: Option<(u64, String, String)>,
        page_size: usize,
    ) -> (Vec<String>, Option<(u64, String, String)>) {
        let after = after.map(|(created_at, chat_id, id)| DeviceSyncCursor {
            created_at,
            chat_id,
            id,
        });
        let (messages, next) =
            collect_device_sync_messages(self, roster_at, after.as_ref(), page_size);
        (
            messages.into_iter().map(|message| message.id).collect(),
            next.map(|cursor| (cursor.created_at, cursor.chat_id, cursor.id)),
        )
    }
}

pub(super) fn metadata_page_packets(core: &AppCore, roster_at: u64, offset: usize) -> Vec<Vec<u8>> {
    let metadata = encode_device_sync_chunks(core.build_device_sync_snapshot(roster_at, false));
    let end = offset
        .saturating_add(DEVICE_SYNC_PAGE_PACKETS)
        .min(metadata.len());
    let mut packets = metadata.get(offset..end).unwrap_or_default().to_vec();
    let next = if end < metadata.len() {
        DeviceSyncPage::Metadata { offset: end }
    } else {
        DeviceSyncPage::Messages { after: None }
    };
    if let Ok(page_end) = serde_json::to_vec(&DeviceSyncPacket::PageEnd {
        v: DEVICE_SYNC_VERSION,
        roster_at,
        next,
    }) {
        packets.push(page_end);
    }
    packets
}

fn page_rank(page: Option<&DeviceSyncPage>) -> Option<(u8, u64, String, String)> {
    match page {
        None => None,
        Some(DeviceSyncPage::Metadata { offset }) => {
            Some((0, *offset as u64, String::new(), String::new()))
        }
        Some(DeviceSyncPage::Messages { after: None }) => {
            Some((1, 0, String::new(), String::new()))
        }
        Some(DeviceSyncPage::Messages {
            after: Some(cursor),
        }) => Some((
            1,
            cursor.created_at,
            cursor.chat_id.clone(),
            cursor.id.clone(),
        )),
    }
}
