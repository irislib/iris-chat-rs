use super::*;

impl AppCore {
    pub(in crate::core) fn refresh_device_sync_on_foreground(&mut self) {
        let peers = self.device_sync.as_ref().map_or_else(Vec::new, |runtime| {
            runtime
                .siblings
                .iter()
                .map(|peer| peer.pubkey().to_string())
                .collect()
        });
        if peers.is_empty() {
            return;
        }
        self.push_debug_log("device_sync.foreground", format!("peers={}", peers.len()));
        for peer in peers {
            // A retained connection only requested an inventory when it opened.
            // Resume from a fresh inventory: a broadcast or an in-flight response
            // may have been missed while inactive. Close both halves of an old
            // round so it cannot hold catch-up until the session's expiry.
            self.clear_device_history(&peer);
            self.request_device_sync_snapshot(&peer, None);
        }
    }

    pub(super) fn reply_device_sync_snapshot(
        &mut self,
        source_pubkey_hex: &str,
        requested_roster_at: u64,
        page: Option<DeviceSyncPage>,
    ) {
        let Some(local_roster_at) = self.device_sync_roster_at() else {
            return;
        };
        let Some(peer_since) = self.device_sync_peer_since(source_pubkey_hex) else {
            return;
        };
        let agreed = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.history.agreed.get(source_pubkey_hex).copied());
        let typed = self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.typed.contains(source_pubkey_hex));
        let floor = self
            .device_history_send_since(source_pubkey_hex)
            .unwrap_or(local_roster_at.max(peer_since));
        let cutoff = agreed.unwrap_or(floor.max(requested_roster_at)).max(floor);
        let DeviceSyncPage::Metadata { offset } =
            page.unwrap_or(DeviceSyncPage::Metadata { offset: 0 });
        let mut packets = metadata_page_packets(self, cutoff, offset);
        if let Some(policy) = self.device_history_policy_packet(source_pubkey_hex) {
            if let Ok(packet) = serde_json::to_vec(&policy) {
                packets.insert(0, packet);
            }
        }
        {
            for packet in &mut packets {
                if let Ok(DeviceSyncPacket::PageEnd {
                    v, roster_at, next, ..
                }) = serde_json::from_slice(packet)
                {
                    if let Ok(updated) = serde_json::to_vec(&DeviceSyncPacket::PageEnd {
                        v,
                        roster_at,
                        next,
                        record_reconcile: typed.then_some(1),
                        private_events: self
                            .private_events_supported(source_pubkey_hex)
                            .then_some(1),
                        history_since: agreed,
                    }) {
                        *packet = updated;
                    }
                }
            }
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
            record_reconcile: Some(1),
            private_events: Some(1),
            history_since: self.device_history_receive_since(source_pubkey_hex),
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
        Some(DeviceSyncPage::Metadata { offset: end })
    } else {
        None
    };
    if let Ok(page_end) = serde_json::to_vec(&DeviceSyncPacket::PageEnd {
        v: DEVICE_SYNC_VERSION,
        roster_at,
        next,
        record_reconcile: Some(1),
        private_events: Some(1),
        history_since: None,
    }) {
        packets.push(page_end);
    }
    packets
}

fn page_rank(page: Option<&DeviceSyncPage>) -> Option<(u8, u64, String, String)> {
    page.map(|DeviceSyncPage::Metadata { offset }| {
        (0, *offset as u64, String::new(), String::new())
    })
}
