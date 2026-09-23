//! Ephemeral, one-to-one calls over the shared authenticated FIPS endpoint.
//! No signaling or media is written to chat history or replayed from a relay.
use super::*;
use crate::state::CallSnapshot;
use std::time::Instant as Clock;
mod receive;
#[cfg(test)]
mod tests;
mod wire;
use wire::PORT;
use wire::{Assembler, Signal};

pub(super) struct ActiveCall {
    pub(super) id: String,
    owner: String,
    targets: Vec<String>,
    peer: Option<String>,
    video: bool,
    offered_video: bool,
    outgoing: bool,
    started: Clock,
    last_received: Clock,
    frames: Assembler,
    audio_seq: u32,
    video_seq: u32,
}
#[derive(Default)]
pub(super) struct CallRuntime {
    pub(super) active: Option<ActiveCall>,
    ended: VecDeque<(String, Clock)>,
}
/// Bounded frame queue: congestion drops old real-time work instead of building
/// an ever-growing send/task backlog. Signaling has its own small path.
pub(super) struct MediaSend {
    peer: fips_core::PeerIdentity,
    packets: Vec<Vec<u8>>,
    queued: Clock,
}
pub(super) async fn start_transport(
    endpoint: Arc<fips_core::FipsEndpoint>,
    sender: Sender<CoreMsg>,
) -> Result<(Sender<MediaSend>, Vec<tokio::task::JoinHandle<()>>), String> {
    let receiver = endpoint
        .register_service_receiver(PORT)
        .await
        .map_err(|e| e.to_string())?;
    let receive_task = tokio::spawn(async move {
        let mut packets = Vec::with_capacity(32);
        while receiver.recv_batch_into(&mut packets, 32).await.is_some() {
            for packet in packets.drain(..) {
                if packet.data.len() > 1129 || sender.len() >= 96 {
                    continue;
                }
                let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::CallPacket {
                    source_pubkey_hex: packet.source_peer.pubkey().to_string(),
                    source_port: packet.source_port,
                    data: packet.data.into_vec(),
                })));
            }
        }
    });
    let (tx, rx) = flume::bounded::<MediaSend>(8);
    let send_task = tokio::spawn(async move {
        while let Ok(frame) = rx.recv_async().await {
            if frame.queued.elapsed() > Duration::from_millis(150) {
                continue;
            }
            for packet in frame.packets {
                if endpoint
                    .send_datagram(frame.peer, PORT, PORT, packet)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    Ok((tx, vec![receive_task, send_task]))
}
impl AppCore {
    fn call_owner(&self, source: &str) -> Option<String> {
        self.app_keys
            .values()
            .find(|keys| {
                keys.created_at_secs > 0
                    && keys.devices.iter().any(|d| d.identity_pubkey_hex == source)
            })
            .map(|keys| keys.owner_pubkey_hex.clone())
    }
    fn call_contact_allowed(&self, owner: &str) -> bool {
        !self.is_owner_blocked(owner)
            && self.can_use_chats()
            && (self
                .preferences
                .accepted_owner_pubkeys
                .iter()
                .any(|v| v == owner)
                || self
                    .threads
                    .get(owner)
                    .is_some_and(|t| t.messages.iter().any(|m| m.is_outgoing)))
    }
    fn call_signal(&self, targets: Vec<String>, signal: Signal) {
        let Some(endpoint) = self.device_sync.as_ref().map(|r| r.endpoint.clone()) else {
            return;
        };
        let Ok(data) = serde_json::to_vec(&signal) else {
            return;
        };
        self.runtime.spawn(async move {
            for peer in targets.into_iter().filter_map(|v| fips_peer_from_hex(&v)) {
                let _ = endpoint.send_datagram(peer, PORT, PORT, data.clone()).await;
            }
        });
    }
    fn signal_active_call(&self, kind: &str) {
        let (Some(active), Some(snapshot)) = (&self.calls.active, &self.state.call) else {
            return;
        };
        let targets = active
            .peer
            .as_ref()
            .map(|p| vec![p.clone()])
            .unwrap_or_else(|| active.targets.clone());
        self.call_signal(
            targets,
            Signal::new(
                kind,
                &active.id,
                match kind {
                    "answer" => active.video,
                    "offer" => active.offered_video,
                    _ => snapshot.video,
                },
                snapshot.muted,
            ),
        );
    }
    fn schedule_call_tick(&self, id: &str) {
        let id = id.to_string();
        let sender = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_secs(1)).await;
            let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::CallTick {
                call_id: id,
            })));
        });
    }
    pub(super) fn start_call(&mut self, owner: &str, video: bool) {
        if self.calls.active.is_some() || !self.can_use_chats() {
            return;
        }
        if !if video {
            self.preferences.video_calls_enabled
        } else {
            self.preferences.voice_calls_enabled
        } {
            return;
        }
        if is_group_chat_id(owner)
            || !self.threads.contains_key(owner)
            || self.is_owner_blocked(owner)
            || self.thread_is_message_request(owner)
        {
            self.state.toast = Some("Open an accepted chat to call".into());
            self.emit_state();
            return;
        }
        let targets = self
            .app_keys
            .get(owner)
            .map(|k| {
                k.devices
                    .iter()
                    .map(|d| d.identity_pubkey_hex.clone())
                    .take(8)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if self.device_sync.is_none() {
            self.reconcile_device_sync();
        }
        if targets.is_empty()
            || self
                .device_sync
                .as_ref()
                .is_none_or(|r| r.calls_tx.is_none())
        {
            self.state.toast = Some("Calling is unavailable. Try again when connected.".into());
            self.emit_state();
            return;
        }
        // Starting a call is an explicit interaction with this contact.
        if !self
            .preferences
            .accepted_owner_pubkeys
            .iter()
            .any(|v| v == owner)
        {
            self.preferences.accepted_owner_pubkeys.push(owner.into());
            self.persist_best_effort();
        }
        let id = rand::random::<[u8; 16]>()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        self.install_call(id.clone(), owner.into(), targets, None, video, video, true);
        self.signal_active_call("offer");
        self.schedule_call_tick(&id);
        self.emit_state();
    }
    #[allow(clippy::too_many_arguments)]
    fn install_call(
        &mut self,
        id: String,
        owner: String,
        targets: Vec<String>,
        peer: Option<String>,
        video: bool,
        offered_video: bool,
        outgoing: bool,
    ) {
        self.state.call = Some(CallSnapshot {
            call_id: id.clone(),
            chat_id: owner.clone(),
            peer_name: self.owner_display_label(&owner),
            phase: if outgoing { "outgoing" } else { "incoming" }.into(),
            video,
            video_capable: video,
            muted: false,
            remote_video: video,
            remote_muted: false,
            started_at_secs: unix_now().get(),
            connected_at_secs: None,
            end_reason: None,
        });
        self.calls.active = Some(ActiveCall {
            id,
            owner,
            targets,
            peer,
            video,
            offered_video,
            outgoing,
            started: Clock::now(),
            last_received: Clock::now(),
            frames: Assembler::default(),
            audio_seq: 0,
            video_seq: 0,
        });
    }
    pub(super) fn answer_call(&mut self, id: &str, voice_only: bool) {
        let (Some(active), Some(snapshot)) = (&mut self.calls.active, &mut self.state.call) else {
            return;
        };
        if active.id != id
            || snapshot.phase != "incoming"
            || (voice_only && !self.preferences.voice_calls_enabled)
        {
            return;
        }
        let video = active.video && !voice_only && self.preferences.video_calls_enabled;
        if !video && !self.preferences.voice_calls_enabled {
            return;
        }
        active.video = video;
        active.last_received = Clock::now();
        snapshot.video &= video;
        snapshot.video_capable = video;
        snapshot.remote_video = video;
        snapshot.phase = "connected".into();
        snapshot.connected_at_secs = Some(unix_now().get());
        self.signal_active_call("answer");
        self.emit_state();
    }
    pub(super) fn end_call(&mut self, id: &str) {
        if self.state.call.as_ref().is_none_or(|c| c.call_id != id) {
            return;
        }
        if self.calls.active.is_some() {
            self.finish_call("Call ended");
        } else {
            self.state.call = None;
            self.emit_state();
        }
    }
    fn remember_ended_call(&mut self, id: String) {
        self.calls.ended.push_back((id, Clock::now()));
        while self.calls.ended.len() > 64 {
            self.calls.ended.pop_front();
        }
    }

    pub(super) fn finish_call(&mut self, reason: &str) {
        self.signal_active_call("end");
        if let Some(active) = self.calls.active.take() {
            self.remember_ended_call(active.id);
            if let Some(snapshot) = &mut self.state.call {
                snapshot.phase = "ended".into();
                snapshot.end_reason = Some(reason.into());
            }
            self.emit_state();
        }
    }
    pub(super) fn set_call_muted(&mut self, muted: bool) {
        let Some(snapshot) = &mut self.state.call else {
            return;
        };
        if snapshot.phase == "ended" {
            return;
        }
        snapshot.muted = muted;
        self.signal_active_call("media_state");
        self.emit_state();
    }
    pub(super) fn set_call_video(&mut self, enabled: bool) {
        let Some(snapshot) = &mut self.state.call else {
            return;
        };
        if snapshot.phase == "ended"
            || !snapshot.video_capable
            || (enabled && !self.preferences.video_calls_enabled)
        {
            return;
        }
        snapshot.video = enabled;
        self.signal_active_call("media_state");
        self.emit_state();
    }
    pub(super) fn set_calls_enabled(&mut self, video: bool, enabled: bool) {
        if video {
            self.preferences.video_calls_enabled = enabled;
        } else {
            self.preferences.voice_calls_enabled = enabled;
        }
        if !enabled && self.calls.active.as_ref().is_some_and(|c| c.video == video) {
            self.finish_call("Calling turned off");
        }
        self.rebuild_persist_and_emit_state();
    }
    pub(super) fn send_call_media(&mut self, id: &str, kind: u8, data: Vec<u8>) {
        let (Some(active), Some(snapshot)) = (&mut self.calls.active, &self.state.call) else {
            return;
        };
        if active.id != id
            || snapshot.phase != "connected"
            || (kind == 1 && snapshot.muted)
            || (kind == 2 && (!snapshot.video || !active.video))
        {
            return;
        }
        let Some(peer) = active.peer.as_ref().and_then(|p| fips_peer_from_hex(p)) else {
            return;
        };
        let seq = if kind == 1 {
            &mut active.audio_seq
        } else {
            &mut active.video_seq
        };
        let packets = wire::encode(id, kind, *seq, &data);
        *seq = seq.wrapping_add(1);
        if packets.is_empty() {
            return;
        }
        if let Some(tx) = self.device_sync.as_ref().and_then(|r| r.calls_tx.as_ref()) {
            let _ = tx.try_send(MediaSend {
                peer,
                packets,
                queued: Clock::now(),
            });
        }
    }
    pub(super) fn call_tick(&mut self, id: &str) {
        let Some(active) = &self.calls.active else {
            return;
        };
        if active.id != id {
            return;
        }
        if !self.call_contact_allowed(&active.owner)
            || active
                .peer
                .as_ref()
                .is_some_and(|p| self.call_owner(p).as_deref() != Some(&active.owner))
        {
            self.finish_call("Call ended");
            return;
        }
        let connected = self
            .state
            .call
            .as_ref()
            .is_some_and(|s| s.phase == "connected");
        if !connected && active.started.elapsed() >= Duration::from_secs(30) {
            self.finish_call("No answer");
            return;
        }
        if connected && active.last_received.elapsed() >= Duration::from_secs(15) {
            self.finish_call("Connection lost");
            return;
        }
        if connected {
            self.signal_active_call("ping");
        } else if active.outgoing {
            self.signal_active_call("offer");
        }
        self.schedule_call_tick(id);
    }
}
