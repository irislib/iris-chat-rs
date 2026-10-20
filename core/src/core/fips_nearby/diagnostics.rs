use super::*;

/// Local debug counters only; never serialized into app state or FFI schemas.
#[derive(Default)]
pub(in crate::core) struct ReceiveDiagnostics {
    arrivals: u64,
    bytes: u64,
    deferred: u64,
    outcomes: BTreeMap<&'static str, u64>,
    last_flush_scheduled: Option<Instant>,
}

pub(in crate::core) fn sample_receive_diagnostic(count: u64) -> bool {
    count <= 8 || count.is_power_of_two()
}

impl AppCore {
    pub(in crate::core) fn observe_fips_nearby_packet_arrival(&mut self, event: &InternalEvent) {
        if let InternalEvent::FipsNearbyPacket {
            source_port, data, ..
        } = event
        {
            let deferred = self.suspended && !self.admit_background_ble_event(event);
            self.record_fips_nearby_arrival(*source_port, data.len(), deferred);
        }
    }

    pub(in crate::core) fn handle_fips_nearby_diagnostic_debug_log(
        &mut self,
        category: &str,
        detail: &str,
    ) -> bool {
        match category {
            "fips_nearby.rx.flush" => {
                self.push_debug_log(category, self.fips_nearby_diagnostic_summary());
                self.persist_debug_snapshot_best_effort();
            }
            "fips_nearby.rx.listener" | "fips_nearby.rx.dequeue" | "fips_nearby.tx.carrier" => {
                self.push_debug_log(category, detail.to_string());
                self.persist_fips_nearby_diagnostics();
            }
            _ => return false,
        }
        true
    }

    pub(in crate::core) fn record_fips_nearby_arrival(
        &mut self,
        port: u16,
        bytes: usize,
        deferred: bool,
    ) {
        if !debug_snapshot_file_writes_enabled() {
            return;
        }
        let counts = &mut self.fips_nearby_receive_diagnostics;
        counts.arrivals = counts.arrivals.saturating_add(1);
        counts.bytes = counts.bytes.saturating_add(bytes as u64);
        counts.deferred = counts.deferred.saturating_add(u64::from(deferred));
        if sample_receive_diagnostic(counts.arrivals)
            || (deferred && sample_receive_diagnostic(counts.deferred))
        {
            let detail = format!(
                "generation={} packets={} bytes={} source_port={port} last_bytes={bytes} suspended={} deferred={} deferred_total={}",
                self.fips_connection_generation, counts.arrivals, counts.bytes, self.suspended, deferred, counts.deferred
            );
            crate::perflog!("fips_nearby.rx.core {detail}");
            self.push_debug_log("fips_nearby.rx.core", detail);
            self.persist_fips_nearby_diagnostics();
        }
    }

    pub(in crate::core) fn record_fips_nearby_outcome(
        &mut self,
        reason: &'static str,
        kind: Option<u16>,
        bytes: usize,
    ) {
        if !debug_snapshot_file_writes_enabled() {
            return;
        }
        let count = self
            .fips_nearby_receive_diagnostics
            .outcomes
            .entry(reason)
            .or_default();
        *count = count.saturating_add(1);
        if sample_receive_diagnostic(*count) {
            let detail = format!("outcome={reason} count={count} kind={kind:?} bytes={bytes}");
            crate::perflog!("fips_nearby.rx.outcome {detail}");
            self.push_debug_log("fips_nearby.rx.outcome", detail);
            self.persist_fips_nearby_diagnostics();
        }
    }

    pub(in crate::core) fn fips_nearby_diagnostic_summary(&self) -> String {
        let counts = &self.fips_nearby_receive_diagnostics;
        format!(
            "generation={} packets={} bytes={} deferred={} outcomes={:?}",
            self.fips_connection_generation,
            counts.arrivals,
            counts.bytes,
            counts.deferred,
            counts.outcomes
        )
    }

    pub(in crate::core) fn persist_fips_nearby_diagnostics(&mut self) {
        // Arrival is observed before the suspend gate. It may update memory,
        // but must not write files or extend an iOS background storage window.
        if self.suspended {
            return;
        }
        self.persist_debug_snapshot_best_effort();
        let now = Instant::now();
        let counts = &mut self.fips_nearby_receive_diagnostics;
        if counts
            .last_flush_scheduled
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(6))
        {
            return;
        }
        counts.last_flush_scheduled = Some(now);
        let sender = self.core_sender.clone();
        self.runtime.spawn(async move {
            // A one-off rejection just after the five-second snapshot floor
            // still needs a later write. The ordinary InternalEvent suspend
            // gate applies to this bounded debug-only refresh too.
            sleep(Duration::from_secs(6)).await;
            let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                category: "fips_nearby.rx.flush".to_string(),
                detail: "debug snapshot refresh".to_string(),
            })));
        });
    }
}

#[cfg(test)]
mod tests;
