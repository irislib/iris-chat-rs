use super::*;
impl AppCore {
    pub(super) fn schedule_call_recovery(&self, id: &str) {
        let call_id = id.to_string();
        let sender = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let _ = sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::CallRecoveryTick { call_id },
            )));
        });
    }
    pub(in crate::core) fn call_recovery_tick(&mut self, id: &str) {
        let (Some(a), Some(s)) = (&mut self.calls.active, &self.state.call) else {
            return;
        };
        if a.id != id || s.phase != "connected" {
            return;
        }
        if let Some(peer) = a.peer.clone() {
            let packets = a.frames.nacks(Clock::now());
            for (seq, missing) in packets {
                let mut signal = Signal::new("nack", id, false, false);
                signal.frame_seq = Some(seq);
                signal.missing = Some(missing);
                self.call_signal(vec![peer.clone()], signal);
            }
        }
        self.schedule_call_recovery(id);
    }
    pub(super) fn receive_call_nack(&mut self, signal: &Signal) {
        let Some(a) = &mut self.calls.active else {
            return;
        };
        let (Some(seq), Some(missing)) = (signal.frame_seq, signal.missing.as_ref()) else {
            return;
        };
        if missing.len() > 64 {
            return;
        }
        if a.retransmit_at.elapsed() >= Duration::from_secs(1) {
            a.retransmit_at = Clock::now();
            a.retransmit_count = 0;
        }
        let Some((_, at, frames)) = a.sent_video.iter().find(|(s, _, _)| *s == seq) else {
            return;
        };
        if at.elapsed() > Duration::from_millis(300) {
            return;
        }
        let Some(peer) = a.peer.as_ref().and_then(|p| fips_peer_from_hex(p)) else {
            return;
        };
        let mut indices = missing.clone();
        indices.sort_unstable();
        indices.dedup();
        let packets: Vec<_> = indices
            .into_iter()
            .filter_map(|i| frames.get(i as usize).cloned())
            .take(128usize.saturating_sub(a.retransmit_count))
            .collect();
        a.retransmit_count += packets.len();
        if !packets.is_empty() {
            if let Some(tx) = self.device_sync.as_ref().and_then(|r| r.calls_tx.as_ref()) {
                let _ = tx.try_send(MediaSend {
                    peer,
                    packets,
                    queued: Clock::now(),
                });
            }
        }
    }

    pub(in crate::core) fn call_bitrate(&self) -> u32 {
        match self.preferences.call_quality.as_str() {
            "high" => 4_000_000,
            "data" => 400_000,
            "custom" => self
                .preferences
                .call_max_bitrate_bps
                .clamp(100_000, 10_000_000),
            _ => 2_000_000,
        }
    }
    pub(in crate::core) fn set_call_quality(&mut self, quality: String, max_bitrate_bps: u32) {
        if !matches!(quality.as_str(), "auto" | "high" | "data" | "custom") {
            return;
        }
        self.preferences.call_quality = quality;
        self.preferences.call_max_bitrate_bps = max_bitrate_bps.clamp(100_000, 10_000_000);
        let cap = self.call_bitrate();
        if let Some(s) = &mut self.state.call {
            s.max_bitrate_bps = cap;
            s.target_bitrate_bps = s.target_bitrate_bps.min(cap);
        }
        self.rebuild_persist_and_emit_state();
    }
    pub(in crate::core) fn set_call_media_connected(&mut self, id: &str, connected: bool) {
        if let Some(s) = &mut self.state.call {
            if s.call_id == id && s.phase == "connected" && s.media_connected != connected {
                s.media_connected = connected;
                if let Some(a) = &mut self.calls.active {
                    if connected {
                        a.media_disconnected_since = None;
                        a.answered
                            .get_or_insert_with(|| (unix_now().get(), Clock::now()));
                    } else {
                        a.media_disconnected_since.get_or_insert_with(Clock::now);
                    }
                }
                self.persist_call_history(None);
                self.emit_state();
            }
        }
    }
    pub(in crate::core) fn send_call_media(
        &mut self,
        id: &str,
        kind: u8,
        timestamp_us: u64,
        key: bool,
        data: Vec<u8>,
    ) {
        let (Some(a), Some(s)) = (&mut self.calls.active, &self.state.call) else {
            return;
        };
        if a.id != id
            || s.phase != "connected"
            || !wire::valid_frame(kind, &data)
            || (kind == 1 && s.muted)
            || (kind == 2 && (!a.video || !s.video))
        {
            return;
        }
        let Some(peer) = a.peer.as_ref().and_then(|p| fips_peer_from_hex(p)) else {
            return;
        };
        let seq = if kind == 1 {
            &mut a.audio_seq
        } else {
            &mut a.video_seq
        };
        let packets = wire::encode(id, kind, *seq, timestamp_us, key, &data);
        *seq = seq.wrapping_add(1);
        if kind == 2 {
            a.sent_video
                .retain(|(_, at, _)| at.elapsed() < Duration::from_millis(300));
            a.sent_video
                .push_back((a.video_seq.wrapping_sub(1), Clock::now(), packets.clone()));
            while a.sent_video.len() > 8
                || a.sent_video
                    .iter()
                    .flat_map(|(_, _, p)| p)
                    .map(Vec::len)
                    .sum::<usize>()
                    > 1_048_576
            {
                a.sent_video.pop_front();
            }
        }
        if let Some(tx) = self.device_sync.as_ref().and_then(|r| r.calls_tx.as_ref()) {
            let _ = tx.try_send(MediaSend {
                peer,
                packets,
                queued: Clock::now(),
            });
        }
    }
    pub(in crate::core) fn request_call_key_frame(&mut self, id: &str) {
        let Some(a) = &mut self.calls.active else {
            return;
        };
        if a.id != id
            || a.last_key_request
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        a.last_key_request = Some(Clock::now());
        self.signal_active_call("keyframe");
    }
    pub(super) fn send_call_feedback(&mut self) {
        let (Some(a), Some(s)) = (&mut self.calls.active, &self.state.call) else {
            return;
        };
        let Some(peer) = a.peer.clone() else {
            return;
        };
        let mut feedback = Signal::new("feedback", &a.id, s.video, s.muted);
        feedback.feedback_seq = Some(a.feedback_seq);
        a.feedback_seq = a.feedback_seq.wrapping_add(1);
        feedback.video_seq = a.frames.highest_video;
        feedback.received_frames = Some(std::mem::take(&mut a.frames.received_frames));
        feedback.received_bytes = Some(std::mem::take(&mut a.frames.received_bytes));
        feedback.interval_ms = Some(a.feedback_at.elapsed().as_millis().clamp(200, 5000) as u32);
        a.feedback_at = Clock::now();
        self.call_signal(vec![peer], feedback);
    }
    pub(super) fn receive_call_feedback(&mut self, feedback: &Signal) {
        let (Some(a), Some(s)) = (&mut self.calls.active, &mut self.state.call) else {
            return;
        };
        let (Some(seq), Some(frames), Some(bytes), Some(interval)) = (
            feedback.feedback_seq,
            feedback.received_frames,
            feedback.received_bytes,
            feedback.interval_ms,
        ) else {
            return;
        };
        if !(200..=5000).contains(&interval)
            || frames > 300
            || bytes > 10_000_000
            || a.last_feedback_seq.is_some_and(|last| {
                seq.wrapping_sub(last) == 0 || seq.wrapping_sub(last) >= 1 << 31
            })
        {
            return;
        }
        let sent = a.video_seq.wrapping_sub(a.feedback_sent_video);
        let expected = feedback.video_seq.map_or(0, |highest| {
            a.feedback_video_seq
                .map_or(highest.wrapping_add(1), |last| highest.wrapping_sub(last))
        });
        if expected >= 1 << 31 {
            return;
        }
        a.last_feedback_seq = Some(seq);
        a.last_feedback_at = Clock::now();
        a.feedback_sent_video = a.video_seq;
        a.feedback_video_seq = feedback.video_seq.or(a.feedback_video_seq);
        // A working control path must not hide complete media loss. No newly
        // received sequence is also loss when we sent video during this interval.
        let expected = if frames == 0 {
            expected.max(sent)
        } else {
            expected
        };
        if !s.video {
            return;
        }
        // The highest observed frame may still be incomplete at a reporting
        // boundary. Carry that debt forward; a late completion pays it back.
        a.feedback_frame_debt =
            (a.feedback_frame_debt + expected.min(1000) as i32 - frames as i32).clamp(0, 1000);
        if expected == 0 {
            return;
        }
        let loss =
            a.feedback_frame_debt.saturating_sub(1).max(0) as f64 / expected.max(frames) as f64;
        let target = if loss > 0.02 || frames == 0 {
            // Complete-frame goodput is conservative after fragment loss. Back
            // off below what arrived and let queues/references recover before
            // probing again. Even one lost reference can freeze H.264 video.
            a.feedback_healthy_ms = 0;
            a.feedback_frame_debt = 0;
            let delivered_bps = (u64::from(bytes) * 8000 / u64::from(interval)) as u32;
            if frames == 0 {
                s.target_bitrate_bps / 2
            } else {
                (s.target_bitrate_bps.saturating_mul(3) / 4)
                    .min(delivered_bps.saturating_mul(85) / 100)
            }
        } else {
            a.feedback_healthy_ms = a.feedback_healthy_ms.saturating_add(interval).min(5000);
            if a.feedback_healthy_ms < 5000 {
                s.target_bitrate_bps
            } else {
                let increase = (u64::from(s.target_bitrate_bps) * 8 / 100 + 10_000)
                    * u64::from(interval)
                    / 1000;
                s.target_bitrate_bps.saturating_add(increase as u32)
            }
        };
        let target = target.clamp(100_000, s.max_bitrate_bps);
        if target != s.target_bitrate_bps {
            s.target_bitrate_bps = target;
            self.emit_state();
        }
    }
    pub(super) fn check_call_feedback(&mut self) {
        let (Some(a), Some(s)) = (&mut self.calls.active, &mut self.state.call) else {
            return;
        };
        if s.video && a.video_seq > 0 && a.last_feedback_at.elapsed() > Duration::from_secs(3) {
            a.feedback_healthy_ms = 0;
            let target = (s.target_bitrate_bps.saturating_mul(3) / 4).max(100_000);
            if target != s.target_bitrate_bps {
                s.target_bitrate_bps = target;
                self.emit_state();
            }
        }
    }
}
