use nostr::{
    Alphabet, Event, EventBuilder, Filter, Kind, PublicKey, SingleLetterTag, Tag, Timestamp,
    UnsignedEvent,
};
use nostr_double_ratchet::GroupSnapshot;
use nostr_double_ratchet_nostr::{APP_KEYS_EVENT_KIND, INVITE_EVENT_KIND, INVITE_RESPONSE_KIND};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const GROUP_ROSTER_FACT_KIND: u32 = 10449;
pub const GROUP_ROSTER_FACT_TYPE: &str = "group_roster_fact";
pub const GROUP_ROSTER_FACT_SCHEMA: u32 = 1;
pub const INVITE_LIST_LABEL: &str = "double-ratchet/invites";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRosterFact {
    pub group_id: String,
    pub signer_pubkey: PublicKey,
    pub snapshot: GroupSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupRosterFactContent {
    #[serde(rename = "type")]
    fact_type: String,
    schema: u32,
    group_id: String,
    snapshot: GroupSnapshot,
}

pub fn group_roster_unsigned_event(
    signer_pubkey: PublicKey,
    snapshot: &GroupSnapshot,
) -> anyhow::Result<UnsignedEvent> {
    validate_group_id(&snapshot.group_id)?;
    let content = GroupRosterFactContent {
        fact_type: GROUP_ROSTER_FACT_TYPE.to_string(),
        schema: GROUP_ROSTER_FACT_SCHEMA,
        group_id: snapshot.group_id.clone(),
        snapshot: snapshot.clone(),
    };
    let content = serde_json::to_string(&content)?;
    let group_id = snapshot.group_id.as_str();
    let revision = snapshot.revision.to_string();
    let updated_at = snapshot.updated_at.get().to_string();
    let tags = vec![
        Tag::parse(["l", group_id])?,
        Tag::parse(["d", group_id])?,
        Tag::parse(["type", GROUP_ROSTER_FACT_TYPE])?,
        Tag::parse(["schema", GROUP_ROSTER_FACT_SCHEMA.to_string().as_str()])?,
        Tag::parse(["revision", revision.as_str()])?,
        Tag::parse(["updated_at", updated_at.as_str()])?,
    ];

    Ok(
        EventBuilder::new(Kind::from(GROUP_ROSTER_FACT_KIND as u16), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(snapshot.updated_at.get()))
            .build(signer_pubkey),
    )
}

pub fn is_group_roster_fact_event(event: &Event) -> bool {
    event.kind.as_u16() as u32 == GROUP_ROSTER_FACT_KIND
}

pub fn parse_group_roster_fact_event(event: &Event) -> anyhow::Result<GroupRosterFact> {
    if event.kind.as_u16() as u32 != GROUP_ROSTER_FACT_KIND {
        anyhow::bail!("not a group roster fact event");
    }
    event.verify()?;

    let content: GroupRosterFactContent = serde_json::from_str(&event.content)?;
    if content.fact_type != GROUP_ROSTER_FACT_TYPE {
        anyhow::bail!("invalid group roster fact type");
    }
    if content.schema != GROUP_ROSTER_FACT_SCHEMA {
        anyhow::bail!("unsupported group roster fact schema");
    }
    validate_group_id(&content.group_id)?;
    if content.snapshot.group_id != content.group_id {
        anyhow::bail!("group roster fact group id mismatch");
    }
    if !event_has_tag_value(event, "l", &content.group_id) {
        anyhow::bail!("missing group roster fact group tag");
    }

    Ok(GroupRosterFact {
        group_id: content.group_id,
        signer_pubkey: event.pubkey,
        snapshot: content.snapshot,
    })
}

pub fn project_group_roster_fact_events<'a>(
    events: impl IntoIterator<Item = &'a Event>,
) -> Vec<GroupSnapshot> {
    let mut latest: BTreeMap<String, (GroupSnapshot, u64, String)> = BTreeMap::new();
    for event in events {
        let Ok(fact) = parse_group_roster_fact_event(event) else {
            continue;
        };
        let ordering = (
            fact.snapshot.revision,
            fact.snapshot.updated_at.get(),
            event.created_at.as_secs(),
            event.id.to_hex(),
        );
        let should_replace = latest
            .get(&fact.group_id)
            .map(|(snapshot, created_at, event_id)| {
                ordering
                    > (
                        snapshot.revision,
                        snapshot.updated_at.get(),
                        *created_at,
                        event_id.clone(),
                    )
            })
            .unwrap_or(true);
        if should_replace {
            latest.insert(
                fact.group_id.clone(),
                (fact.snapshot, event.created_at.as_secs(), event.id.to_hex()),
            );
        }
    }
    latest
        .into_values()
        .map(|(snapshot, _, _)| snapshot)
        .collect()
}

pub fn build_group_roster_fact_filter<'a>(
    group_ids: impl IntoIterator<Item = &'a String>,
    authors: Vec<PublicKey>,
) -> Filter {
    let group_ids = group_ids
        .into_iter()
        .filter(|group_id| !group_id.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut filter = Filter::new().kind(Kind::from(GROUP_ROSTER_FACT_KIND as u16));
    if !group_ids.is_empty() {
        filter = filter.custom_tags(SingleLetterTag::lowercase(Alphabet::D), group_ids);
    }
    if !authors.is_empty() {
        filter = filter.authors(authors);
    }
    filter
}

pub fn build_protocol_discovery_filters(
    roster_authors: Vec<PublicKey>,
    invite_authors: Vec<PublicKey>,
    limit: usize,
) -> Vec<Filter> {
    let mut filters = Vec::new();
    if !roster_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors(roster_authors),
        );
    }
    if !invite_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_EVENT_KIND as u16))
                .authors(invite_authors.clone())
                .custom_tag(SingleLetterTag::lowercase(Alphabet::L), INVITE_LIST_LABEL)
                .limit(limit),
        );
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .authors(invite_authors)
                .limit(limit),
        );
    }
    filters
}

fn event_has_tag_value(event: &Event, tag_name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let tag = tag.as_slice();
        tag.first().is_some_and(|name| name == tag_name)
            && tag.get(1).is_some_and(|candidate| candidate == value)
    })
}

fn validate_group_id(group_id: &str) -> anyhow::Result<()> {
    if group_id.trim().is_empty() {
        anyhow::bail!("group id is empty");
    }
    Ok(())
}
