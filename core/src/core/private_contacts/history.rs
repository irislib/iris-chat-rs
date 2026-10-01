use super::*;
use nostr_sdk::prelude::ReqExitPolicy;

const PAGE_SIZE: usize = 200;
const MAX_PAGE_SIZE: usize = 6_400;

struct Cursor {
    until: Option<u64>,
    limit: usize,
}
impl Default for Cursor {
    fn default() -> Self {
        Self {
            until: None,
            limit: PAGE_SIZE,
        }
    }
}
impl Cursor {
    fn filter(&self, base: &Filter) -> Filter {
        let filter = base.clone().limit(self.limit);
        self.until.map_or(filter.clone(), |until| {
            filter.until(Timestamp::from_secs(until))
        })
    }

    fn advance(&mut self, timestamps: &[u64]) -> anyhow::Result<bool> {
        if timestamps.len() < self.limit {
            return Ok(false);
        }
        let oldest = timestamps
            .iter()
            .min()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("empty history page"))?;
        if self.until == Some(oldest) {
            anyhow::ensure!(
                self.limit < MAX_PAGE_SIZE,
                "private contact timestamp page saturated"
            );
            self.limit = (self.limit * 2).min(MAX_PAGE_SIZE);
        } else {
            // Inclusive overlap is essential: stepping below the oldest second
            // can permanently skip other contacts created in that same second.
            self.until = Some(oldest);
            self.limit = PAGE_SIZE;
        }
        Ok(true)
    }
}

pub(super) async fn recover(client: &Client, filter: Filter, tx: Sender<CoreMsg>) {
    let relays = client.relays().await;
    let mut complete = !relays.is_empty();
    let mut seen = HashSet::new();
    for relay in relays.into_values() {
        let mut cursor = Cursor::default();
        for page in 0..500 {
            // Query each server directly. A shared cache can hide an older
            // replaceable head and turn a full history page into a short one.
            let events = match relay
                .fetch_events(
                    cursor.filter(&filter),
                    Duration::from_secs(8),
                    ReqExitPolicy::ExitOnEOSE,
                )
                .await
            {
                Ok(events) => events.iter().cloned().collect::<Vec<_>>(),
                Err(_) => {
                    complete = false;
                    break;
                }
            };
            let timestamps = events
                .iter()
                .map(|event| event.created_at.as_secs())
                .collect::<Vec<_>>();
            let unseen = events
                .into_iter()
                .filter(|event| seen.insert(event.id))
                .collect::<Vec<_>>();
            if !unseen.is_empty()
                && tx
                    .send(CoreMsg::Internal(Box::new(
                        InternalEvent::FetchCatchUpEvents(unseen),
                    )))
                    .is_err()
            {
                return;
            }
            match cursor.advance(&timestamps) {
                Ok(false) => break,
                Ok(true) if page < 499 => {}
                _ => {
                    complete = false;
                    break;
                }
            }
        }
    }
    if !complete {
        let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "private_contacts.history_partial".into(),
            detail: "Some saved contact history could not be read; reconnect to retry.".into(),
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_contact_history_overlaps_and_widens_without_skipping_same_second_heads() {
        let history = (0..450)
            .map(|id| (id, 50))
            .chain([(450, 40), (451, 30)])
            .collect::<Vec<_>>();
        let mut cursor = Cursor::default();
        let mut found = HashSet::new();
        for _ in 0..10 {
            let page = history
                .iter()
                .filter(|(_, time)| cursor.until.is_none_or(|until| *time <= until))
                .take(cursor.limit)
                .copied()
                .collect::<Vec<_>>();
            found.extend(page.iter().map(|(id, _)| *id));
            if !cursor
                .advance(&page.iter().map(|(_, time)| *time).collect::<Vec<_>>())
                .unwrap()
            {
                break;
            }
        }
        assert_eq!(found.len(), history.len());
        assert_eq!(cursor.limit, 800);
    }

    #[test]
    fn private_contact_history_saturation_is_partial_instead_of_skipping_a_boundary() {
        let mut cursor = Cursor {
            until: Some(50),
            limit: MAX_PAGE_SIZE,
        };
        assert!(cursor.advance(&vec![50; MAX_PAGE_SIZE]).is_err());
        assert_eq!(cursor.until, Some(50));
        assert!(!Cursor::default().advance(&[]).unwrap());
    }
}
