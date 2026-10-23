// Included only by Linux tests. The observer is outside engine snapshots so
// rollback clones do not copy the audit, and shipping builds have no hooks.
#[derive(Default)]
struct RetrySearchAudit {
    generations: std::collections::HashMap<NdrDevicePubkey, u64>,
    seen: HashSet<(NdrDevicePubkey, u64, String)>,
    starts: usize,
    repeated: usize,
    changes: usize,
    phase_work: BTreeMap<&'static str, std::time::Duration>,
}

pub(super) fn audit_group_retry_decrypt_elapsed(elapsed: std::time::Duration) {
    record_retry_phase("decrypt_cursor_nested", elapsed);
}

fn record_retry_phase(name: &'static str, elapsed: std::time::Duration) {
    RETRY_SEARCH_AUDIT.with(|slot| {
        if let Some(audit) = slot.borrow_mut().as_mut() {
            *audit.phase_work.entry(name).or_default() += elapsed;
        }
    });
}

pub(super) struct RetryPhaseTimer {
    name: &'static str,
    started: Option<std::time::Instant>,
}

impl RetryPhaseTimer {
    pub(super) fn start(name: &'static str) -> Self {
        Self {
            name,
            started: RETRY_SEARCH_AUDIT
                .with(|slot| slot.borrow().as_ref().map(|_| std::time::Instant::now())),
        }
    }
}

impl Drop for RetryPhaseTimer {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            record_retry_phase(self.name, started.elapsed());
        }
    }
}

thread_local! {
    static RETRY_SEARCH_AUDIT: std::cell::RefCell<Option<RetrySearchAudit>> = const {
        std::cell::RefCell::new(None)
    };
}

pub(super) fn audit_group_retry_input_changes(changed: &HashSet<NdrDevicePubkey>) {
    RETRY_SEARCH_AUDIT.with(|slot| {
        if let Some(audit) = slot.borrow_mut().as_mut() {
            for author in changed {
                *audit.generations.entry(*author).or_default() += 1;
                audit.changes += 1;
            }
        }
    });
}

pub(super) fn audit_group_retry_attempt(author: NdrDevicePubkey, fingerprint: &str) {
    RETRY_SEARCH_AUDIT.with(|slot| {
        if let Some(audit) = slot.borrow_mut().as_mut() {
            let generation = *audit.generations.entry(author).or_default();
            audit.starts += 1;
            if !audit
                .seen
                .insert((author, generation, fingerprint.to_owned()))
            {
                audit.repeated += 1;
            }
        }
    });
}

struct RetrySearchAuditGuard;

impl RetrySearchAuditGuard {
    fn start() -> Self {
        RETRY_SEARCH_AUDIT.with(|slot| {
            assert!(slot.replace(Some(RetrySearchAudit::default())).is_none());
        });
        Self
    }

    fn counts(&self) -> (usize, usize, usize) {
        RETRY_SEARCH_AUDIT.with(|slot| {
            let audit = slot.borrow();
            let audit = audit.as_ref().unwrap();
            (audit.starts, audit.repeated, audit.changes)
        })
    }

    fn take_phase_seconds(&self) -> BTreeMap<&'static str, f64> {
        RETRY_SEARCH_AUDIT.with(|slot| {
            let mut measured = std::mem::take(&mut slot.borrow_mut().as_mut().unwrap().phase_work);
            // A fully exhausted search has no cursor work. Keep the same
            // diagnostic schema for that legitimate quiet phase.
            [
                "retirement",
                "group_inputs",
                "group_fanouts",
                "inbound",
                "remote_sends",
                "local_siblings",
                "flush",
                "decrypt_cursor_nested",
            ]
            .into_iter()
            .map(|phase| {
                (
                    phase,
                    measured.remove(phase).unwrap_or_default().as_secs_f64(),
                )
            })
            .collect()
        })
    }
}

impl Drop for RetrySearchAuditGuard {
    fn drop(&mut self) {
        RETRY_SEARCH_AUDIT.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn retry_search_audit_keeps_author_generations_across_observations() {
    let author = ndr_device(Keys::generate().public_key());
    let other = ndr_device(Keys::generate().public_key());
    let audit = RetrySearchAuditGuard::start();
    audit_group_retry_input_changes(&HashSet::from([author]));
    audit_group_retry_attempt(author, "synthetic candidate");
    assert_eq!(audit.counts(), (1, 0, 1));
    // Observing a slice boundary or changing another author must not forget
    // that the original candidate already exhausted these exact inputs.
    audit_group_retry_input_changes(&HashSet::from([other]));
    audit_group_retry_attempt(author, "synthetic candidate");
    assert_eq!(audit.counts(), (2, 1, 2));
    audit_group_retry_input_changes(&HashSet::from([author]));
    audit_group_retry_attempt(author, "synthetic candidate");
    assert_eq!(audit.counts(), (3, 1, 3));
    record_retry_phase("group_inputs", std::time::Duration::from_millis(3));
    audit_group_retry_decrypt_elapsed(std::time::Duration::from_millis(1));
    audit_group_retry_decrypt_elapsed(std::time::Duration::from_millis(1));
    let phases = audit.take_phase_seconds();
    assert_eq!(phases.len(), 8);
    assert_eq!(phases["group_inputs"], 0.003);
    assert_eq!(phases["decrypt_cursor_nested"], 0.002);
    assert_eq!(phases["flush"], 0.0);
    record_retry_phase("remote_sends", std::time::Duration::from_millis(1));
    let no_decrypt = audit.take_phase_seconds();
    assert_eq!(no_decrypt.len(), 8);
    assert_eq!(no_decrypt["remote_sends"], 0.001);
    assert_eq!(no_decrypt["group_inputs"], 0.0);
    assert_eq!(no_decrypt["decrypt_cursor_nested"], 0.0);
    assert_eq!(audit.counts(), (3, 1, 3));
    drop(audit);
    let restarted = RetrySearchAuditGuard::start();
    assert_eq!(restarted.counts(), (0, 0, 0));
}
