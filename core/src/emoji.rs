//! Shared emoji catalog and search aliases used by the picker on
//! every platform that ships its own grid (iOS/macOS, Android,
//! Windows). Linux uses the native GTK EmojiChooser and ignores
//! this catalog.

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
    vec![
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
    ]
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
}
