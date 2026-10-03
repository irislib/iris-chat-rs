use crate::core::{fallback_profile_name_for_identity, profile_name_for_identity};
use nostr::PublicKey;

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PersonNamePresentation {
    pub name: String,
    pub is_fallback: bool,
}

/// Display-only repair of cached identifiers and generated names. Explicit
/// profile names and nicknames are distinguished by provenance, not word shape.
#[uniffi::export]
pub fn present_person_name(
    display_name: String,
    identity: String,
    explicit_name: Option<String>,
) -> PersonNamePresentation {
    let label = display_name.trim();
    let identity = identity.trim();
    let generated = fallback_profile_name_for_identity(identity);
    let explicit = !label.is_empty() && explicit_name.as_deref().map(str::trim) == Some(label);
    let normalized = PublicKey::parse(identity).ok().map(|key| key.to_hex());
    let is_fallback = !explicit
        && (label.is_empty()
            || (!identity.is_empty()
                && (profile_name_for_identity(label, identity).is_none()
                    || label == generated
                    || label == legacy_generated_name(identity)
                    || normalized
                        .as_deref()
                        .is_some_and(|key| label == legacy_generated_name(key)))));
    PersonNamePresentation {
        name: if is_fallback { generated } else { label.into() },
        is_fallback,
    }
}

// Recognize only the exact old generated value for this identity. These labels
// may still be cached by an existing app; no stored profile is rewritten.
fn legacy_generated_name(identity: &str) -> String {
    const ADJECTIVES: [&str; 12] = [
        "Amber", "Bright", "Calm", "Clear", "Golden", "Lunar", "Nova", "Quiet", "Silver", "Solar",
        "Velvet", "Wild",
    ];
    const NOUNS: [&str; 12] = [
        "Aurora", "Comet", "Echo", "Falcon", "Harbor", "Listener", "Otter", "Raven", "Signal",
        "Sparrow", "Tide", "Voyager",
    ];
    if identity.is_empty() {
        return "Quiet Listener".into();
    }
    let hash = identity.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    }) as usize;
    let adjective = ADJECTIVES.get(hash % ADJECTIVES.len()).unwrap_or(&"Quiet");
    let noun = NOUNS
        .get((hash / ADJECTIVES.len()) % NOUNS.len())
        .unwrap_or(&"Listener");
    format!("{adjective} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::ToBech32;

    #[test]
    fn identifiers_and_old_generated_names_share_a_stable_animal_fallback() {
        let owner = "ab".repeat(32);
        let npub = PublicKey::parse(&owner).unwrap().to_bech32().unwrap();
        for label in [
            "",
            "   ",
            &owner,
            "abababab…abababab",
            &npub,
            "Golden Listener",
        ] {
            let value = present_person_name(label.into(), owner.clone(), None);
            assert_eq!(value.name, "Golden Hare");
            assert!(value.is_fallback);
        }
        assert_eq!(
            present_person_name(String::new(), npub, None),
            present_person_name(String::new(), owner, None)
        );
    }

    #[test]
    fn explicit_names_are_preserved_even_when_they_match_a_generated_name_or_id() {
        let owner = "ab".repeat(32);
        for name in ["Alice", "Mum", "Golden Hare", "Amber Fox", &owner] {
            let value = present_person_name(name.into(), owner.clone(), Some(name.into()));
            assert_eq!(value.name, name);
            assert!(!value.is_fallback);
        }
        for name in ["Alice", "Amber Fox", "Note to Self"] {
            assert!(!present_person_name(name.into(), owner.clone(), None).is_fallback);
        }
        assert_eq!(
            present_person_name(String::new(), String::new(), None).name,
            "Quiet Otter"
        );
    }
}
