use super::*;

pub(super) fn enqueue_control_retries(
    retries: &Arc<Mutex<HashMap<String, ControlRecord>>>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
) {
    let records = retries
        .lock()
        .map(|mut records| std::mem::take(&mut *records))
        .unwrap_or_default();
    for (peer, record) in records {
        if enqueue_pending(
            pending,
            pending_count,
            peer.clone(),
            frame(&record.bytes),
            true,
            true,
        ) {
            continue;
        }
        if let Ok(mut records) = retries.lock() {
            store_control_record(&mut records, peer, record);
        }
    }
}

pub(super) fn store_control_record(
    records: &mut HashMap<String, ControlRecord>,
    peer: String,
    incoming: ControlRecord,
) {
    match records.entry(peer) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(incoming);
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            if control_precedes(&incoming, entry.get()) {
                entry.insert(incoming);
            }
        }
    }
}

fn control_precedes(incoming: &ControlRecord, existing: &ControlRecord) -> bool {
    match (&incoming.rank, &existing.rank) {
        (None, Some(_)) => true,
        (Some(incoming), Some(existing)) => incoming < existing,
        _ => false,
    }
}

pub(super) fn enqueue_dirty_resyncs(
    dirty: &Arc<Mutex<HashSet<String>>>,
    pending: &mut HashMap<String, VecDeque<PendingRecord>>,
    pending_count: &mut usize,
    resync_required: &[u8],
) {
    let peers = dirty
        .lock()
        .map(|mut peers| std::mem::take(&mut *peers))
        .unwrap_or_default();
    for peer in peers {
        if enqueue_pending(
            pending,
            pending_count,
            peer.clone(),
            frame(resync_required),
            true,
            true,
        ) {
            continue;
        }
        if let Ok(mut peers) = dirty.lock() {
            peers.insert(peer);
        }
    }
}

pub(super) fn mark_dirty(dirty: &Arc<Mutex<HashSet<String>>>, peer: String) {
    if let Ok(mut peers) = dirty.lock() {
        peers.insert(peer);
    }
}
