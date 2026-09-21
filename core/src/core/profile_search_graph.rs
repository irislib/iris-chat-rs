use super::*;
use std::sync::Mutex;

type CachedGraph = (String, [u8; 32], Arc<SocialGraph>);
static PEOPLE_GRAPH: OnceLock<Mutex<Option<CachedGraph>>> = OnceLock::new();

pub(super) fn load_people_graph(
    conn: &Connection,
    owner: Option<&str>,
) -> anyhow::Result<Option<Arc<SocialGraph>>> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    let bytes = conn
        .query_row(
            "SELECT social_graph FROM user_discovery_state WHERE id = 1 AND owner_pubkey_hex = ?1",
            [owner],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten();
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let fingerprint = hashtree_core::sha256(&bytes);
    let mut cached = PEOPLE_GRAPH
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow::anyhow!("people graph cache mutex poisoned"))?;
    if let Some((cached_owner, cached_fingerprint, graph)) = cached.as_ref() {
        if cached_owner == owner && *cached_fingerprint == fingerprint {
            return Ok(Some(graph.clone()));
        }
    }
    let Some(graph) = SocialGraph::from_binary(owner, &bytes).ok().map(Arc::new) else {
        return Ok(None);
    };
    *cached = Some((owner.to_string(), fingerprint, graph.clone()));
    Ok(Some(graph))
}

pub(super) fn graph_hides_person(owner: &str, graph: Option<&SocialGraph>) -> bool {
    // Personal opinions at the closest known distance take precedence, including
    // direct follows and direct mutes. Unknown people can still be discovered.
    if let Some(graph) = graph {
        let has_opinion = graph
            .get_followers_by_user(owner)
            .into_iter()
            .chain(graph.get_user_muted_by(owner))
            .any(|author| graph.get_follow_distance(&author) < 1_000);
        if has_opinion {
            return graph.is_overmuted(owner, 1.0);
        }
    }
    DEFAULT_SOCIAL_GRAPH
        .get()
        .and_then(Option::as_ref)
        .is_some_and(|graph| graph.is_overmuted(owner, 1.0))
}
