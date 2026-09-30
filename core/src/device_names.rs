// Keep this vocabulary and FNV-1a mapping aligned with web/deviceLabels.ts.
const ADJECTIVES: [&str; 32] = [
    "Amber", "Bright", "Calm", "Clever", "Cosmic", "Cozy", "Curious", "Dancing", "Dapper",
    "Dreamy", "Gentle", "Golden", "Happy", "Jolly", "Kind", "Lively", "Lucky", "Lunar", "Mellow",
    "Merry", "Misty", "Nimble", "Noble", "Quiet", "Silver", "Sleepy", "Snowy", "Solar", "Sunny",
    "Swift", "Velvet", "Wise",
];
const ANIMALS: [&str; 32] = [
    "Badger", "Bear", "Beaver", "Bison", "Cat", "Crane", "Deer", "Dolphin", "Dove", "Falcon",
    "Finch", "Fox", "Gecko", "Heron", "Koala", "Lemur", "Lynx", "Marten", "Moth", "Otter", "Owl",
    "Panda", "Puffin", "Rabbit", "Robin", "Seal", "Sparrow", "Swan", "Tiger", "Turtle", "Whale",
    "Wren",
];

pub(crate) fn meaningful_device_name(label: Option<&str>) -> Option<&str> {
    label.map(str::trim).filter(|label| {
        !label.is_empty()
            && !["linked device", "this device", "unnamed device"]
                .iter()
                .any(|generic| label.eq_ignore_ascii_case(generic))
    })
}

pub(crate) fn unnamed_device_name(device_id: &str) -> String {
    let hash = device_id
        .to_ascii_lowercase()
        .bytes()
        .fold(2_166_136_261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    format!(
        "{} {}",
        ADJECTIVES[(hash & 31) as usize],
        ANIMALS[((hash >> 5) & 31) as usize]
    )
}
