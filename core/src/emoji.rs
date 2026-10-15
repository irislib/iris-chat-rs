//! Shared emoji catalog and search aliases for native pickers and typed
//! shortcode completion. Linux keeps its native GTK EmojiChooser and uses
//! these aliases for completion in the message composer.

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct EmojiCategorySnapshot {
    pub name: String,
    pub emojis: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct EmojiAliasSnapshot {
    pub emoji: String,
    pub keywords: String,
}

fn cat(name: &str, emojis: &[&str]) -> EmojiCategorySnapshot {
    EmojiCategorySnapshot {
        name: name.to_string(),
        emojis: emojis.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn alias(emoji: &str, keywords: &str) -> EmojiAliasSnapshot {
    EmojiAliasSnapshot {
        emoji: emoji.to_string(),
        keywords: keywords.to_string(),
    }
}

#[uniffi::export]
pub fn iris_emoji_catalog() -> Vec<EmojiCategorySnapshot> {
    vec![
        cat(
            "Smileys",
            &[
                "😀",
                "😃",
                "😄",
                "😁",
                "😆",
                "😅",
                "🤣",
                "😂",
                "🙂",
                "🙃",
                "🫠",
                "😉",
                "😊",
                "😇",
                "🥰",
                "😍",
                "🤩",
                "😘",
                "😗",
                "☺️",
                "😚",
                "😙",
                "🥲",
                "😋",
                "😛",
                "😜",
                "🤪",
                "😝",
                "🤑",
                "🤗",
                "🤭",
                "🫢",
                "🫣",
                "🤫",
                "🤔",
                "🫡",
                "🤐",
                "🤨",
                "😐",
                "😑",
                "😶",
                "🫥",
                "😶‍🌫️",
                "😏",
                "😒",
                "🙄",
                "😬",
                "😮‍💨",
                "🤥",
                "😌",
                "😔",
                "😪",
                "🤤",
                "😴",
                "😷",
                "🤒",
                "🤕",
                "🤢",
                "🤮",
                "🤧",
                "🥵",
                "🥶",
                "🥴",
                "😵",
                "😵‍💫",
                "🤯",
                "🤠",
                "🥳",
                "🥸",
                "😎",
                "🤓",
                "🧐",
                "😕",
                "🫤",
                "😟",
                "🙁",
                "☹️",
                "😮",
                "😯",
                "😲",
                "😳",
                "🥺",
                "🥹",
                "😦",
                "😧",
                "😨",
                "😰",
                "😥",
                "😢",
                "😭",
                "😱",
                "😖",
                "😣",
                "😞",
                "😓",
                "😩",
                "😫",
                "🥱",
                "😤",
                "😡",
                "😠",
                "🤬",
                "😈",
                "👿",
                "💀",
                "☠️",
                "💩",
                "🤡",
                "👹",
                "👺",
                "👻",
                "👽",
                "👾",
                "🤖",
                "😺",
                "😸",
                "😹",
                "😻",
                "😼",
                "😽",
                "🙀",
                "😿",
                "😾",
            ],
        ),
        cat(
            "Hearts",
            &[
                "❤️",
                "🧡",
                "💛",
                "💚",
                "💙",
                "💜",
                "🖤",
                "🤍",
                "🤎",
                "💖",
                "💗",
                "💓",
                "💞",
                "💕",
                "💘",
                "💝",
                "💟",
                "♥️",
                "💔",
                "❣️",
                "❤️‍🔥",
                "❤️‍🩹",
            ],
        ),
        cat(
            "Hands",
            &[
                "👍", "👎", "👌", "🤌", "🤏", "✌️", "🤞", "🫰", "🤟", "🤘", "🤙", "👈", "👉", "👆",
                "👇", "☝️", "✋", "🤚", "🖐", "🖖", "👋", "🤝", "🙏", "👏", "🙌", "🫶", "💪", "🫵",
                "🫱", "🫲",
            ],
        ),
        cat(
            "Animals",
            &[
                "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸",
                "🐵", "🙈", "🙉", "🙊", "🐔", "🐧", "🐦", "🦅", "🦉", "🦄", "🐝", "🦋", "🐞", "🐢",
                "🐍", "🦖", "🐙", "🦀", "🐬", "🐳", "🦈",
            ],
        ),
        cat(
            "Food",
            &[
                "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🫐", "🍒", "🍑", "🥭", "🍍",
                "🥥", "🥝", "🍅", "🥑", "🥕", "🌽", "🍆", "🥔", "🍕", "🍔", "🍟", "🌭", "🍿", "🥪",
                "🌮", "🌯", "🍣", "🍜", "🍝", "🍦", "🍩", "🍪", "🎂", "🍰", "☕", "🍵", "🍺", "🥂",
                "🍷", "🥃",
            ],
        ),
        cat(
            "Activities",
            &[
                "⚽", "🏀", "🏈", "⚾", "🥎", "🎾", "🏐", "🏉", "🎱", "🪀", "🏓", "🏸", "🥅", "🏒",
                "🏑", "🥍", "🏏", "🪃", "🥊", "🥋", "🎽", "⛸", "🥌", "🛷", "🪂", "🏋️", "🤸", "🤺",
                "🏇", "⛷", "🏂", "🏌️", "🏄", "🚣", "🏊", "🤽", "🚴", "🚵", "🎯", "🎮", "🎲", "🎼",
                "🎤", "🎧", "🎷", "🎸", "🥁",
            ],
        ),
        cat(
            "Travel",
            &[
                "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐", "🛻", "🚚", "🚛", "🚜",
                "🛵", "🏍", "🛺", "🚲", "🛴", "🛹", "🚂", "✈️", "🚀", "🛸", "🛶", "⛵", "🚢", "🚁",
                "🗺", "🗽", "🗼", "🏰", "🎡", "🎢", "🎠", "🏖", "🏝", "🏔", "🌋", "🏕", "🌄", "🌅",
                "🌌",
            ],
        ),
        cat(
            "Objects",
            &[
                "📱", "💻", "⌨️", "🖥", "🖨", "🖱", "💾", "💿", "📷", "📸", "📹", "🎥", "📺", "📻",
                "📞", "☎️", "🔌", "🔋", "💡", "🔦", "🕯", "🧯", "🛢", "💵", "💰", "💳", "💎", "⚖️",
                "🔧", "🔨", "🛠", "⛏", "🪛", "🪚", "🔩", "⚙️", "🧱", "⛓", "🧲", "🔫", "💣", "🧨",
            ],
        ),
        cat(
            "Symbols",
            &[
                "✅", "❎", "✔️", "❌", "⭕", "🚫", "⚠️", "🔱", "☑️", "💯", "🔥", "✨", "🌟", "⭐",
                "🌈", "☀️", "🌙", "⚡", "☄️", "💥", "🌊", "💧", "💦", "🎉", "🎊", "🎁", "🎀", "🎈",
                "🪅", "🍾", "🥇", "🥈", "🥉", "🏆", "🎖", "🏅", "💤", "💭", "🗯", "💬", "🆗", "🆕",
                "🆒", "🆓", "🆙", "🔝", "♻️", "☮️", "✝️", "☪️", "🕉", "☸️", "✡️", "☯️", "☦️",
            ],
        ),
    ]
}

#[uniffi::export]
pub fn iris_emoji_search_aliases() -> Vec<EmojiAliasSnapshot> {
    let search_aliases = vec![
        alias("😂", "laugh laughing lol haha"),
        alias("🤣", "laugh laughing lol haha rolling"),
        alias("😊", "smile smiling happy"),
        alias("🙂", "smile smiling happy"),
        alias("😍", "love heart eyes"),
        alias("🥰", "love hearts"),
        alias("😘", "kiss love"),
        alias("😢", "sad tear crying"),
        alias("😭", "sad cry crying"),
        alias("😠", "angry mad"),
        alias("🤬", "angry mad swearing"),
        alias("🙏", "pray praying thanks thank you please"),
        alias("👏", "clap applause"),
        alias("🙌", "hooray yay hands"),
        alias("❤️", "love heart red"),
        alias("♥️", "love heart red"),
        alias("🔥", "fire lit hot"),
        alias("🎉", "party celebrate celebration"),
        alias("🎊", "party celebrate celebration"),
        alias("✨", "sparkle sparkles"),
        alias("✅", "yes check done"),
        alias("❌", "no cross x"),
        alias("👀", "eyes look watching"),
        alias("💯", "hundred perfect"),
        alias("😮", "wow surprised shock shocked open mouth"),
        alias("😯", "wow surprised hushed"),
        alias("😲", "wow surprised astonished shock"),
        alias("🤯", "mind blown shock wow"),
        alias("😱", "scream shock shocked wow"),
        alias("🤔", "thinking hmm think"),
        alias("🤢", "sick gross nauseated"),
        alias("🤮", "sick gross vomit puke"),
        alias("🥱", "yawn yawning tired bored"),
        alias("😴", "sleep sleeping tired"),
        alias("🤓", "nerd nerdy glasses"),
        alias("🧐", "monocle inspect curious"),
        alias("💀", "skull dead"),
        alias("👻", "ghost spooky"),
        alias("🤡", "clown"),
        alias("👽", "alien"),
        alias("🤖", "robot bot"),
        alias("💩", "poo poop"),
        alias("😈", "devil smiling devil"),
        alias("👍", "thumbs up like yes"),
        alias("👎", "thumbs down dislike no"),
        alias("🥺", "pleading puppy eyes"),
    ];
    // Shortcode names come first so native composers can show one stable label.
    // Keep natural-language search words after them for the existing pickers.
    include_str!("../assets/emoji-shortcodes.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(emoji, shortcodes)| {
            let extra = search_aliases.iter().find(|entry| entry.emoji == emoji);
            let keywords = match extra {
                Some(entry) => format!("{shortcodes} {}", entry.keywords),
                None => shortcodes.to_string(),
            };
            EmojiAliasSnapshot {
                emoji: emoji.to_string(),
                keywords,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_wow_emoji() {
        let catalog = iris_emoji_catalog();
        let smileys = catalog
            .iter()
            .find(|c| c.name == "Smileys")
            .expect("Smileys category");
        assert!(
            smileys.emojis.iter().any(|e| e == "😮"),
            "Smileys must include the open-mouth (wow) emoji"
        );
    }

    #[test]
    fn aliases_route_wow_to_open_mouth() {
        let aliases = iris_emoji_search_aliases();
        let hit = aliases
            .iter()
            .find(|a| a.emoji == "😮")
            .expect("alias entry for open-mouth emoji");
        assert!(hit.keywords.contains("wow"));
    }

    #[test]
    fn every_picker_emoji_has_a_unique_completion_label() {
        let aliases = iris_emoji_search_aliases();
        let mut labels = std::collections::HashSet::new();
        for emoji in iris_emoji_catalog().into_iter().flat_map(|c| c.emojis) {
            let entry = aliases.iter().find(|entry| entry.emoji == emoji).unwrap();
            let label = entry.keywords.split_whitespace().next().unwrap();
            assert!(labels.insert(label), "duplicate shortcode: {label}");
            assert!(label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c)));
        }
        assert_eq!(labels.len(), aliases.len());
    }

    #[test]
    fn familiar_shortcodes_keep_the_same_emoji_on_every_platform() {
        let aliases = iris_emoji_search_aliases();
        for (emoji, expected) in [
            ("😄", "smile"),
            ("😂", "joy"),
            ("❤️", "heart"),
            ("👍", "thumbsup"),
            ("🔥", "fire"),
        ] {
            let entry = aliases.iter().find(|entry| entry.emoji == emoji).unwrap();
            assert_eq!(entry.keywords.split_whitespace().next(), Some(expected));
        }
        assert!(aliases
            .iter()
            .find(|entry| entry.emoji == "👍")
            .unwrap()
            .keywords
            .split_whitespace()
            .any(|word| word == "+1"));
    }
}
