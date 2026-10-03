use super::*;
use rusqlite::OptionalExtension;

const PREFIX: &str = "iris-chat-history-pair-v1:";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::core) struct HistoryTransfer {
    pub(in crate::core) outbound: bool,
    pub(in crate::core) link_at: u64,
    pub(in crate::core) link_id: String,
    pub(in crate::core) since: u64,
    pub(in crate::core) complete: bool,
    #[serde(default)]
    pub(in crate::core) policy_known: bool,
    #[serde(default)]
    pub(in crate::core) imported: u64,
    #[serde(default)]
    pub(in crate::core) total: Option<u64>,
    #[serde(default)]
    pub(in crate::core) fallback: bool,
    #[serde(default)]
    pub(in crate::core) fallback_failed: bool,
}

impl AppCore {
    pub(in crate::core) fn create_device_history_transfer(
        &self,
        device: PublicKey,
        include_history: bool,
        link_id: String,
    ) -> anyhow::Result<()> {
        let peer = device.to_hex();
        let link_at = self
            .device_sync_peer_since(&peer)
            .ok_or_else(|| anyhow::anyhow!("Device is not authorized"))?;
        self.store_device_history_transfer(
            &peer,
            Some(&HistoryTransfer {
                outbound: true,
                link_at,
                link_id,
                since: if include_history { 0 } else { link_at },
                complete: !include_history,
                policy_known: true,
                imported: 0,
                total: None,
                fallback: false,
                fallback_failed: false,
            }),
        )
    }

    pub(in crate::core) fn record_device_history_approver(
        &self,
        peer: &str,
        link_at: u64,
        link_id: String,
    ) -> anyhow::Result<()> {
        if self
            .device_history_transfer(peer)
            .is_some_and(|record| record.link_at == link_at && record.link_id == link_id)
        {
            return Ok(());
        }
        self.store_device_history_transfer(
            peer,
            Some(&HistoryTransfer {
                outbound: false,
                link_at,
                link_id,
                since: link_at,
                complete: false,
                policy_known: false,
                imported: 0,
                total: None,
                fallback: false,
                fallback_failed: false,
            }),
        )
    }

    pub(in crate::core) fn store_device_history_transfer(
        &self,
        peer: &str,
        transfer: Option<&HistoryTransfer>,
    ) -> anyhow::Result<()> {
        let logged = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let shared = self.app_store.shared();
        let conn = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let key = format!(
            "{PREFIX}{}:{}:{peer}",
            logged.owner_pubkey.to_hex(),
            logged.device_keys.public_key().to_hex()
        );
        if let Some(transfer) = transfer {
            conn.execute("INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key, serde_json::to_string(transfer)?])?;
        } else {
            conn.execute("DELETE FROM app_meta WHERE key=?1", [key])?;
        }
        Ok(())
    }

    pub(in crate::core) fn device_history_transfer(&self, peer: &str) -> Option<HistoryTransfer> {
        let logged = self.logged_in.as_ref()?;
        let shared = self.app_store.shared();
        let conn = shared.lock().ok()?;
        let key = format!(
            "{PREFIX}{}:{}:{peer}",
            logged.owner_pubkey.to_hex(),
            logged.device_keys.public_key().to_hex()
        );
        let value: Option<String> = conn
            .query_row("SELECT value FROM app_meta WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .ok()?;
        let transfer: HistoryTransfer = serde_json::from_str(&value?).ok()?;
        self.device_sync_peer_since(peer)?;
        let current_link = if transfer.outbound {
            self.device_sync_peer_since(peer)
        } else {
            self.device_sync_roster_at()
        }?;
        (transfer.link_at == current_link).then_some(transfer)
    }

    pub(in crate::core) fn device_history_send_since(&self, peer: &str) -> Option<u64> {
        self.device_history_transfer(peer)
            .filter(|record| record.outbound)
            .map(|record| {
                if record.complete {
                    record.link_at
                } else {
                    record.since
                }
            })
    }

    pub(in crate::core) fn device_history_receive_since(&self, peer: &str) -> Option<u64> {
        self.device_history_transfer(peer)
            .filter(|record| !record.outbound)
            .map(|record| {
                if record.complete {
                    record.link_at
                } else {
                    record.since
                }
            })
    }

    pub(super) fn device_history_policy_packet(&self, peer: &str) -> Option<DeviceSyncPacket> {
        let record = self.device_history_transfer(peer)?;
        if record.outbound {
            Some(DeviceSyncPacket::HistoryPolicy {
                v: 1,
                link_at: record.link_at,
                link_id: record.link_id.clone(),
                since: if record.complete {
                    record.link_at
                } else {
                    record.since
                },
            })
        } else if record.complete && record.since == 0 {
            Some(DeviceSyncPacket::HistoryComplete {
                v: 1,
                link_at: record.link_at,
                link_id: record.link_id.clone(),
            })
        } else {
            None
        }
    }

    pub(in crate::core) fn handle_device_history_policy(
        &mut self,
        peer: &str,
        link_at: u64,
        since: u64,
        link_id: String,
    ) {
        let Some(mut record) = self.device_history_transfer(peer).filter(|record| {
            !record.outbound && record.link_at == link_at && record.link_id == link_id
        }) else {
            return;
        };
        if since != 0 && since != link_at {
            return;
        }
        if record.complete {
            if record.since == 0 {
                self.send_history_packets(
                    peer,
                    vec![DeviceSyncPacket::HistoryComplete {
                        v: 1,
                        link_at,
                        link_id: link_id.clone(),
                    }],
                );
            }
            return;
        }
        // A private policy belongs to the authenticated approver recorded during pairing.
        // Once learned, reconnects cannot silently widen or change the user's choice.
        if record.policy_known {
            return;
        }
        record.since = since;
        record.policy_known = true;
        record.complete = since == link_at;
        if self
            .store_device_history_transfer(peer, Some(&record))
            .is_ok()
        {
            self.update_device_history_progress(
                peer,
                crate::DeviceHistorySyncPhase::Waiting,
                record.total,
            );
        }
    }

    pub(in crate::core) fn handle_device_history_complete(
        &mut self,
        peer: &str,
        link_at: u64,
        link_id: String,
    ) {
        let Some(mut record) = self.device_history_transfer(peer).filter(|record| {
            record.outbound && record.link_at == link_at && record.link_id == link_id
        }) else {
            return;
        };
        record.complete = true;
        if self
            .store_device_history_transfer(peer, Some(&record))
            .is_ok()
        {
            self.send_history_packets(
                peer,
                vec![DeviceSyncPacket::HistoryComplete {
                    v: 1,
                    link_at,
                    link_id: link_id.clone(),
                }],
            );
        }
    }

    pub(super) fn complete_device_history_import(
        &mut self,
        peer: &str,
    ) -> Option<DeviceSyncPacket> {
        let mut record = self
            .device_history_transfer(peer)
            .filter(|record| !record.outbound && record.since == 0)?;
        record.complete = true;
        self.store_device_history_transfer(peer, Some(&record))
            .ok()?;
        self.update_device_history_progress(
            peer,
            crate::DeviceHistorySyncPhase::Complete,
            Some(record.imported),
        );
        Some(DeviceSyncPacket::HistoryComplete {
            v: 1,
            link_at: record.link_at,
            link_id: record.link_id.clone(),
        })
    }

    pub(in crate::core) fn update_device_history_progress(
        &mut self,
        peer: &str,
        phase: crate::DeviceHistorySyncPhase,
        total: Option<u64>,
    ) {
        let Some(mut record) = self
            .device_history_transfer(peer)
            .filter(|record| !record.outbound && record.policy_known && record.since == 0)
        else {
            return;
        };
        record.total = total;
        if self
            .store_device_history_transfer(peer, Some(&record))
            .is_err()
        {
            return;
        }
        let next = Some(crate::DeviceHistorySyncSnapshot {
            phase,
            imported_messages: record.imported,
            total_messages: total,
        });
        if self.state.device_history_sync != next {
            self.state.device_history_sync = next;
            self.emit_state();
        }
    }

    pub(in crate::core) fn add_device_history_imported(&mut self, peer: &str, count: u64) -> bool {
        let Some(mut record) = self
            .device_history_transfer(peer)
            .filter(|record| !record.outbound && record.since == 0 && !record.complete)
        else {
            return false;
        };
        record.imported = record.imported.saturating_add(count);
        self.store_device_history_transfer(peer, Some(&record))
            .is_ok()
    }

    pub(in crate::core) fn restore_device_history_progress(&mut self) {
        self.state.device_history_sync = None;
        let Some(logged) = self.logged_in.as_ref() else {
            return;
        };
        let peers = self
            .app_keys
            .get(&logged.owner_pubkey.to_hex())
            .map(|keys| {
                keys.devices
                    .iter()
                    .map(|device| device.identity_pubkey_hex.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for peer in peers {
            if let Some(record) = self
                .device_history_transfer(&peer)
                .filter(|record| !record.outbound && record.policy_known && record.since == 0)
            {
                self.state.device_history_sync = Some(crate::DeviceHistorySyncSnapshot {
                    phase: if record.complete {
                        crate::DeviceHistorySyncPhase::Complete
                    } else {
                        crate::DeviceHistorySyncPhase::Waiting
                    },
                    imported_messages: record.imported,
                    total_messages: record.total,
                });
                break;
            }
        }
    }
}

impl AppCore {
    pub(super) fn begin_device_history_fallback(&mut self, peer: &str) {
        let Some(mut record) = self
            .device_history_transfer(peer)
            .filter(|record| !record.outbound && !record.complete && record.since == 0)
        else {
            return;
        };
        record.fallback = true;
        record.fallback_failed = false;
        if self
            .store_device_history_transfer(peer, Some(&record))
            .is_ok()
        {
            self.update_device_history_progress(
                peer,
                crate::DeviceHistorySyncPhase::Transferring,
                None,
            );
        }
    }

    pub(super) fn apply_device_history_snapshot(
        &mut self,
        peer: &str,
        mut snapshot: DeviceSyncSnapshot,
    ) {
        let fallback = self
            .device_history_transfer(peer)
            .filter(|record| !record.outbound && !record.complete && record.fallback);
        for message in &mut snapshot.messages {
            if fallback
                .as_ref()
                .is_none_or(|record| record.since != 0 || message.created_at >= record.link_at)
            {
                message.legacy_reactions = None;
            }
        }
        let incoming = snapshot
            .messages
            .iter()
            .filter(|message| {
                fallback
                    .as_ref()
                    .is_some_and(|record| message.created_at < record.link_at)
            })
            .map(|message| {
                (
                    message.chat_id.clone(),
                    message.id.clone(),
                    self.app_store
                        .message_exists_or_deleted(&message.chat_id, Some(&message.id), None)
                        .unwrap_or(true),
                )
            })
            .collect::<Vec<_>>();
        self.apply_device_sync_snapshot(snapshot, self.device_history_receive_since(peer));
        if let Some(mut record) = fallback.filter(|_| !incoming.is_empty()) {
            self.persist_best_effort_inner();
            let durable = incoming.iter().all(|(chat, id, _)| {
                self.app_store
                    .message_exists_or_deleted(chat, Some(id), None)
                    .unwrap_or(false)
            });
            record.fallback_failed |= !durable;
            if durable {
                record.imported = record.imported.saturating_add(
                    incoming.iter().filter(|(_, _, existed)| !existed).count() as u64,
                );
            }
            if self
                .store_device_history_transfer(peer, Some(&record))
                .is_ok()
            {
                self.update_device_history_progress(
                    peer,
                    crate::DeviceHistorySyncPhase::Transferring,
                    None,
                );
            }
        }
    }

    pub(super) fn finish_device_history_fallback(
        &mut self,
        peer: &str,
        link_at: u64,
        link_id: &str,
    ) {
        let Some(record) = self.device_history_transfer(peer).filter(|record| {
            !record.outbound
                && !record.complete
                && record.fallback
                && record.link_at == link_at
                && record.link_id == link_id
        }) else {
            return;
        };
        if !record.fallback_failed {
            if let Some(packet) = self.complete_device_history_import(peer) {
                self.send_history_packets(peer, vec![packet]);
            }
        } else {
            self.update_device_history_progress(peer, crate::DeviceHistorySyncPhase::Waiting, None);
        }
        self.start_device_history(peer, link_at);
    }
}

impl AppCore {
    pub(in crate::core) fn invalidate_removed_device_history(
        &mut self,
        owner: PublicKey,
        previous: Option<&KnownAppKeys>,
        next: &KnownAppKeys,
    ) -> anyhow::Result<()> {
        let Some(logged) = self
            .logged_in
            .as_ref()
            .filter(|logged| logged.owner_pubkey == owner)
        else {
            return Ok(());
        };
        let removed = previous
            .into_iter()
            .flat_map(|known| &known.devices)
            .filter(|device| {
                !next
                    .devices
                    .iter()
                    .any(|current| current.identity_pubkey_hex == device.identity_pubkey_hex)
            })
            .map(|device| device.identity_pubkey_hex.clone())
            .collect::<Vec<_>>();
        if removed.is_empty() {
            return Ok(());
        }
        let local = logged.device_keys.public_key().to_hex();
        let all = removed.contains(&local);
        let prefix = format!("{PREFIX}{}:{local}:", owner.to_hex());
        {
            let shared = self.app_store.shared();
            let mut conn = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            if all {
                tx.execute(
                    "DELETE FROM app_meta WHERE key LIKE ?1",
                    [format!("{prefix}%")],
                )?;
            } else {
                for peer in &removed {
                    tx.execute(
                        "DELETE FROM app_meta WHERE key=?1",
                        [format!("{prefix}{peer}")],
                    )?;
                }
            }
            tx.commit()?;
        }
        // A later authorization of the same key, even at the same second, must
        // never revive the previous link operation's old-history permission.
        for peer in &removed {
            self.clear_device_history(peer);
        }
        if all {
            if let Some(runtime) = &mut self.device_sync {
                runtime.history = Default::default();
            }
        }
        self.restore_device_history_progress();
        Ok(())
    }
}
