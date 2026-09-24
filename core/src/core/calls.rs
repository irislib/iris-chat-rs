//! Ephemeral, one-to-one calls over the shared authenticated FIPS endpoint.
//! Signaling and media stay ephemeral; only local summaries enter chat history.
use super::*;
use crate::state::CallSnapshot;
use std::time::Instant as Clock;
mod history;
mod media;
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
    answered: Option<(u64, Clock)>,
    last_received: Clock,
    frames: Assembler,
    media_disconnected_since: Option<Clock>,
    audio_seq: u32,
    video_seq: u32,
    feedback_seq: u32,
    feedback_at: Clock,
    last_feedback_at: Clock,
    last_feedback_seq: Option<u32>,
    feedback_video_seq: Option<u32>,
    feedback_sent_video: u32,
    last_key_request: Option<Clock>,
    last_peer_key_request: Option<Clock>,
    sent_video: VecDeque<(u32, Clock, Vec<Vec<u8>>)>,
    retransmit_at: Clock,
    retransmit_count: usize,
}
#[derive(Default)]
pub(super) struct CallRuntime {
    pub(super) active: Option<ActiveCall>,
    ended: VecDeque<(String, Clock)>,
    dispositions: VecDeque<(String, Vec<String>, Signal, Clock)>,
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
        // A maximum-size video frame needs 239 datagrams. The old shared
        // 96-message cutoff dropped its tail even on a lossless local link.
        // Batch dispatch also avoids waking the core once per fragment. At most
        // 512 datagrams (~570 KiB) can wait, independently of other events.
        // Count packets, since a batch may contain fewer than 32 of them.
        let slots = Arc::new(tokio::sync::Semaphore::new(512));
        while receiver.recv_batch_into(&mut packets, 32).await.is_some() {
            let received_at = Clock::now();
            let mut media = Vec::with_capacity(packets.len());
            for packet in packets.drain(..) {
                if packet.data.len() > 1138 {
                    continue;
                }
                let source = packet.source_peer.pubkey().to_string();
                let data = packet.data.into_vec();
                if data.starts_with(b"IC03") {
                    media.push((source, packet.source_port, data));
                } else if sender.len() < 96 {
                    let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::CallPacket {
                        source_pubkey_hex: source,
                        source_port: packet.source_port,
                        data,
                    })));
                }
            }
            if !media.is_empty() {
                if let Ok(permit) = slots.clone().try_acquire_many_owned(media.len() as u32) {
                    let _ =
                        sender.send(CoreMsg::Internal(Box::new(InternalEvent::CallMediaBatch {
                            packets: media,
                            received_at,
                            _permit: permit,
                        })));
                }
            }
        }
    });
    let (tx, rx) = flume::bounded::<MediaSend>(64);
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
    fn remember_call_disposition(&mut self, owner: String, peers: Vec<String>, signal: Signal) {
        self.calls
            .dispositions
            .push_back((owner, peers, signal, Clock::now()));
        while self.calls.dispositions.len() > 64 {
            self.calls.dispositions.pop_front();
        }
    }
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
        let started_at_secs = self.chat_activity_after_deletion(&owner, unix_now().get());
        self.state.call = Some(CallSnapshot {
            outgoing,
            target_bitrate_bps: self.call_bitrate().min(1_200_000),
            key_frame_generation: 0,
            media_connected: false,
            max_bitrate_bps: self.call_bitrate(),
            call_id: id.clone(),
            chat_id: owner.clone(),
            peer_name: self.owner_display_label(&owner),
            phase: if outgoing { "outgoing" } else { "incoming" }.into(),
            video,
            video_capable: video,
            muted: false,
            remote_video: video,
            remote_muted: false,
            started_at_secs,
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
            answered: None,
            last_received: Clock::now(),
            frames: Assembler::default(),
            media_disconnected_since: None,
            audio_seq: 0,
            video_seq: 0,
            feedback_seq: 0,
            feedback_at: Clock::now(),
            last_feedback_at: Clock::now(),
            last_feedback_seq: None,
            feedback_video_seq: None,
            feedback_sent_video: 0,
            last_key_request: None,
            last_peer_key_request: None,
            sent_video: VecDeque::new(),
            retransmit_at: Clock::now(),
            retransmit_count: 0,
        });
        self.persist_call_history(None);
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
        self.schedule_call_recovery(id);
        if let Some(active) = &mut self.calls.active {
            active.media_disconnected_since = Some(Clock::now());
        }
        self.persist_call_history(None);
        self.emit_state();
    }
    pub(super) fn end_call(&mut self, id: &str) {
        if self.state.call.as_ref().is_none_or(|c| c.call_id != id) {
            return;
        }
        if self.calls.active.is_some() {
            self.finish_call(
                if self
                    .state
                    .call
                    .as_ref()
                    .is_some_and(|c| c.phase == "incoming")
                {
                    "Call declined"
                } else {
                    "Call ended"
                },
            );
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
        let declined = reason == "Call declined";
        if declined {
            if let Some(active) = &self.calls.active {
                let targets = active
                    .peer
                    .as_ref()
                    .map(|p| vec![p.clone()])
                    .unwrap_or_else(|| active.targets.clone());
                let mut signal = Signal::new(
                    if active.outgoing { "end" } else { "reject" },
                    &active.id,
                    active.video,
                    false,
                );
                signal.reason = Some("declined".into());
                self.call_signal(targets.clone(), signal.clone());
                self.remember_call_disposition(active.owner.clone(), targets, signal);
            }
        } else {
            self.signal_active_call("end");
        }
        self.persist_call_history(match reason {
            "Call declined" => Some("declined"),
            "Answered on another device" => Some("answered_elsewhere"),
            _ => None,
        });
        if let Some(active) = self.calls.active.take() {
            self.remember_ended_call(active.id);
            if let Some(snapshot) = &mut self.state.call {
                snapshot.phase = "ended".into();
                snapshot.media_connected = false;
                snapshot.end_reason = Some(reason.into());
            }
            self.rebuild_state();
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
        if !connected
            && !active.outgoing
            && active.last_received.elapsed() >= Duration::from_secs(10)
        {
            self.finish_call("Connection lost");
            return;
        }
        if connected
            && active
                .media_disconnected_since
                .is_some_and(|since| since.elapsed() >= Duration::from_secs(30))
        {
            self.finish_call("Couldn’t connect the call");
            return;
        }
        if connected && active.last_received.elapsed() >= Duration::from_secs(15) {
            self.finish_call("Connection lost");
            return;
        }
        if connected {
            self.send_call_feedback();
            self.check_call_feedback();
            self.signal_active_call("ping");
        } else if active.outgoing {
            self.signal_active_call("offer");
        } else {
            // Recover a lost answered-elsewhere/cancel notice while ringing.
            self.signal_active_call("ping");
        }
        self.schedule_call_tick(id);
    }
}
