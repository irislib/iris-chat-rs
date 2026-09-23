use super::*;
impl AppCore {
    pub(in crate::core) fn handle_call_packet(&mut self, source: &str, port: u16, data: &[u8]) {
        if port != PORT {
            return;
        }
        let Some(owner) = self.call_owner(source) else {
            return;
        };
        if !self.call_contact_allowed(&owner) {
            return;
        }
        if data.starts_with(b"IC03") {
            let (Some(active), Some(snapshot)) = (&mut self.calls.active, &self.state.call) else {
                return;
            };
            if snapshot.phase != "connected" || active.peer.as_deref() != Some(source) {
                return;
            }
            if let Some(frame) = active.frames.receive(&active.id, data, Clock::now()) {
                if (frame.kind == 1 && snapshot.remote_muted)
                    || (frame.kind == 2 && (!active.video || !snapshot.remote_video))
                {
                    return;
                }
                active.last_received = Clock::now();
                if self.update_tx.len() < 32 {
                    let _ = self.update_tx.send(AppUpdate::CallMedia {
                        call_id: active.id.clone(),
                        kind: frame.kind,
                        sequence: frame.seq,
                        timestamp_us: frame.timestamp_us,
                        key_frame: frame.key,
                        data: frame.data,
                    });
                }
            }
            return;
        }
        let Some(signal) = Signal::decode(data) else {
            return;
        };
        self.calls
            .dispositions
            .retain(|(_, _, _, at)| at.elapsed() < Duration::from_secs(120));
        if matches!(signal.kind.as_str(), "offer" | "answer" | "ping") {
            if let Some((_, _, reply, _)) =
                self.calls
                    .dispositions
                    .iter()
                    .rev()
                    .find(|(known_owner, peers, reply, _)| {
                        known_owner == &owner
                            && reply.call_id == signal.call_id
                            && peers.iter().any(|p| p == source)
                    })
            {
                self.call_signal(vec![source.into()], reply.clone());
                return;
            }
        }
        self.calls
            .ended
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(120));
        if self.calls.ended.iter().any(|(id, _)| id == &signal.call_id) {
            if !matches!(signal.kind.as_str(), "end" | "reject") {
                self.call_signal(
                    vec![source.into()],
                    Signal::new("end", &signal.call_id, false, false),
                );
            }
            return;
        }
        // A datagram cancellation may overtake the original offer. Keep it
        // even without a live session so that delayed ringing cannot revive it.
        if signal.kind == "end"
            && self
                .calls
                .active
                .as_ref()
                .is_none_or(|c| c.id != signal.call_id)
        {
            self.remember_ended_call(signal.call_id);
            return;
        }
        if signal.kind == "offer" {
            if self
                .calls
                .active
                .as_ref()
                .is_none_or(|call| call.id != signal.call_id)
                && self.call_history_contains(&owner, &signal.call_id)
            {
                self.call_signal(
                    vec![source.into()],
                    Signal::new("end", &signal.call_id, false, false),
                );
                return;
            }
            if let Some(active) = self.calls.active.as_mut() {
                if active.id == signal.call_id && active.peer.as_deref() == Some(source) {
                    active.last_received = Clock::now();
                    if self
                        .state
                        .call
                        .as_ref()
                        .is_some_and(|c| c.phase == "connected")
                    {
                        self.signal_active_call("answer");
                    }
                    return;
                }
                // Deterministic glare resolution when both contacts call at once.
                if active.outgoing && active.owner == owner && signal.call_id < active.id {
                    self.finish_call("Call ended");
                } else {
                    self.call_signal(
                        vec![source.into()],
                        Signal::new("reject", &signal.call_id, false, false),
                    );
                    self.remember_ended_call(signal.call_id);
                    return;
                }
            }
            let video = signal.video.unwrap_or(false) && self.preferences.video_calls_enabled;
            if !video && !self.preferences.voice_calls_enabled {
                self.call_signal(
                    vec![source.into()],
                    Signal::new("reject", &signal.call_id, false, false),
                );
                self.remember_ended_call(signal.call_id);
                return;
            }
            self.install_call(
                signal.call_id.clone(),
                owner,
                vec![source.into()],
                Some(source.into()),
                video,
                signal.video.unwrap_or(false),
                false,
            );
            if let Some(snapshot) = &mut self.state.call {
                snapshot.remote_muted = signal.muted.unwrap_or(false);
            }
            self.schedule_call_tick(&signal.call_id);
            self.emit_state();
            return;
        }
        let Some(active) = &self.calls.active else {
            return;
        };
        if active.id != signal.call_id
            || active.owner != owner
            || !active.targets.iter().any(|p| p == source)
        {
            return;
        }
        let connected = self
            .state
            .call
            .as_ref()
            .is_some_and(|s| s.phase == "connected");
        if active.peer.as_ref().is_some_and(|peer| peer != source) {
            if active.outgoing && connected && matches!(signal.kind.as_str(), "answer" | "ping") {
                self.call_signal(
                    vec![source.into()],
                    Signal::answered_elsewhere(&active.id, active.video),
                );
            }
            return;
        }
        match signal.kind.as_str() {
            "answer" if active.outgoing && !connected => {
                let rejected: Vec<String> = active
                    .targets
                    .iter()
                    .filter(|p| p.as_str() != source)
                    .cloned()
                    .collect();
                let disposition = Signal::answered_elsewhere(
                    &signal.call_id,
                    active.offered_video && signal.video.unwrap_or(false),
                );
                self.call_signal(rejected.clone(), disposition.clone());
                self.remember_call_disposition(owner.clone(), rejected, disposition);
                if let (Some(active), Some(snapshot)) =
                    (&mut self.calls.active, &mut self.state.call)
                {
                    active.peer = Some(source.into());
                    active.video = active.offered_video && signal.video.unwrap_or(false);
                    active.last_received = Clock::now();
                    snapshot.phase = "connected".into();
                    snapshot.video &= active.video;
                    snapshot.video_capable = active.video;
                    snapshot.remote_video = active.video;
                    snapshot.remote_muted = signal.muted.unwrap_or(false);
                    snapshot.connected_at_secs = Some(unix_now().get());
                }
                self.schedule_call_recovery(&signal.call_id);
                self.signal_active_call("ping");
                if let Some(active) = &mut self.calls.active {
                    active.media_disconnected_since = Some(Clock::now());
                }
                self.persist_call_history(None);
                self.emit_state();
            }
            "reject" | "end" => {
                if signal.reason.as_deref() == Some("declined") {
                    self.finish_call("Call declined");
                    return;
                }
                if signal.kind == "end"
                    && !active.outgoing
                    && signal.reason.as_deref() == Some("answered_elsewhere")
                {
                    if let Some(active) = &mut self.calls.active {
                        if let Some(video) = signal.video {
                            active.video = video;
                            active.offered_video = video;
                        }
                    }
                    self.finish_call("Answered on another device");
                    return;
                }
                if active.outgoing && !connected {
                    if let Some(active) = &mut self.calls.active {
                        active.targets.retain(|p| p != source);
                        if !active.targets.is_empty() {
                            return;
                        }
                    }
                }
                self.finish_call(if signal.kind == "reject" {
                    "Call declined"
                } else {
                    "Call ended"
                });
            }
            "nack" if connected => self.receive_call_nack(&signal),
            "feedback" if connected => self.receive_call_feedback(&signal),
            "keyframe" if connected => {
                if let (Some(active), Some(snapshot)) =
                    (&mut self.calls.active, &mut self.state.call)
                {
                    if snapshot.video
                        && active
                            .last_peer_key_request
                            .is_none_or(|at| at.elapsed() >= Duration::from_millis(800))
                    {
                        active.last_peer_key_request = Some(Clock::now());
                        snapshot.key_frame_generation =
                            snapshot.key_frame_generation.wrapping_add(1);
                        self.emit_state();
                    }
                }
            }
            "ping" | "pong" | "media_state" if connected => {
                if let Some(active) = &mut self.calls.active {
                    active.last_received = Clock::now();
                }
                if let Some(snapshot) = &mut self.state.call {
                    let video =
                        snapshot.video_capable && signal.video.unwrap_or(snapshot.remote_video);
                    let muted = signal.muted.unwrap_or(snapshot.remote_muted);
                    let changed = snapshot.remote_video != video || snapshot.remote_muted != muted;
                    snapshot.remote_video = video;
                    snapshot.remote_muted = muted;
                    if changed {
                        self.emit_state();
                    }
                }
                if signal.kind == "ping" {
                    self.signal_active_call("pong");
                }
            }
            _ => {}
        }
    }
}
