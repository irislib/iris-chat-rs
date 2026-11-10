//! Privacy cleanup for self-contained, non-fragmented MP4/QuickTime uploads.
//!
//! Rewrite metadata boxes as zero-filled `free` boxes of exactly the same size,
//! preserving sample offsets, encoded audio/video, timing and display matrices.
//! Remove timed metadata tracks. Validate audio/video sample maps, then zero every
//! unreferenced byte in media-data boxes, including removed timed GPS tracks.
//! This does not redact information visible/audible in the media or codec data.
//! Unsupported containers/layouts fail before any bytes enter the upload store.

use std::ops::Range;
use std::path::Path;

#[path = "video_metadata/tracks.rs"]
mod tracks;

pub(super) const ERROR: &str = "Could not remove video details. Try a standard MP4 copy.";
type Result<T> = std::result::Result<T, &'static str>;
const MAX_CHUNKS: usize = 100_000;

pub(super) fn detect(prefix: &[u8], path: &Path) -> Result<bool> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if prefix
        .get(4..8)
        .is_some_and(|kind| matches!(kind, b"ftyp" | b"moov" | b"mdat" | b"wide"))
    {
        return Ok(true);
    }
    if matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "3gp" | "3g2") {
        return Ok(true);
    }
    if matches!(
        ext.as_str(),
        "webm" | "mkv" | "avi" | "flv" | "wmv" | "mpeg" | "mpg" | "ogv" | "ts" | "mts" | "m2ts"
    ) || prefix.starts_with(b"\x1a\x45\xdf\xa3")
        || prefix.starts_with(b"FLV")
        || (prefix.starts_with(b"RIFF") && prefix.get(8..12) == Some(b"AVI "))
    {
        return Err(ERROR);
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug)]
struct Atom {
    start: usize,
    body: usize,
    end: usize,
    kind: [u8; 4],
}

impl Atom {
    fn payload(self) -> Range<usize> {
        self.body..self.end
    }
}

fn bytes(data: &[u8], at: usize, count: usize) -> Result<&[u8]> {
    data.get(at..at.checked_add(count).ok_or(ERROR)?)
        .ok_or(ERROR)
}

fn uint(data: &[u8], at: usize, count: usize) -> Result<usize> {
    let mut value = 0u64;
    for b in bytes(data, at, count)? {
        value = (value << 8) | u64::from(*b);
    }
    usize::try_from(value).map_err(|_| ERROR)
}

fn atoms(data: &[u8], range: Range<usize>) -> Result<Vec<Atom>> {
    let mut at = range.start;
    let mut result = Vec::new();
    while at < range.end {
        if range.end - at < 8 || result.len() >= 100_000 {
            return Err(ERROR);
        }
        let size = uint(data, at, 4)?;
        let kind = bytes(data, at + 4, 4)?.try_into().map_err(|_| ERROR)?;
        let (size, header) = match size {
            0 => (range.end - at, 8),
            1 => (uint(data, at + 8, 8)?, 16),
            n => (n, 8),
        };
        if size < header || size > range.end - at {
            return Err(ERROR);
        }
        result.push(Atom {
            start: at,
            body: at + header,
            end: at + size,
            kind,
        });
        at += size;
    }
    Ok(result)
}

fn single(list: &[Atom], kind: &[u8; 4]) -> Result<Atom> {
    let mut found = list.iter().filter(|a| &a.kind == kind);
    let atom = *found.next().ok_or(ERROR)?;
    if found.next().is_some() {
        return Err(ERROR);
    }
    Ok(atom)
}

fn zero(out: &mut [u8], range: Range<usize>) -> Result<()> {
    out.get_mut(range).ok_or(ERROR)?.fill(0);
    Ok(())
}

fn discard(out: &mut [u8], atom: Atom) -> Result<()> {
    out.get_mut(atom.start + 4..atom.start + 8)
        .ok_or(ERROR)?
        .copy_from_slice(b"free");
    zero(out, atom.payload())
}

fn dates(out: &mut [u8], atom: Atom) -> Result<()> {
    let len = match out.get(atom.body) {
        Some(0) => 8,
        Some(1) => 16,
        _ => return Err(ERROR),
    };
    if atom.end - atom.body < 4 + len {
        return Err(ERROR);
    }
    zero(out, atom.body + 4..atom.body + 4 + len)
}

fn handler(data: &[u8], mdia: Atom) -> Result<[u8; 4]> {
    let hdlr = single(&atoms(data, mdia.payload())?, b"hdlr")?;
    if hdlr.end - hdlr.body < 24 {
        return Err(ERROR);
    }
    bytes(data, hdlr.body + 8, 4)?.try_into().map_err(|_| ERROR)
}

pub(super) fn clean(data: &[u8]) -> Result<Vec<u8>> {
    let top = atoms(data, 0..data.len())?;
    let moov = single(&top, b"moov")?;
    let children = atoms(data, moov.payload())?;
    let mut video_track = false;
    for track in children.iter().filter(|a| &a.kind == b"trak") {
        let mdia = single(&atoms(data, track.payload())?, b"mdia")?;
        video_track |= &handler(data, mdia)? == b"vide";
    }
    // Detect by content, even for a renamed movie; leave audio-only recordings
    // untouched. Audio privacy is a separate policy from outgoing video cleanup.
    if !video_track {
        return Ok(data.to_vec());
    }
    let media: Vec<_> = top.iter().filter(|a| &a.kind == b"mdat").copied().collect();
    if media.is_empty() {
        return Err(ERROR);
    }
    let mut out = data.to_vec();
    for atom in &top {
        match &atom.kind {
            b"ftyp" | b"moov" | b"mdat" => {}
            b"free" | b"skip" | b"wide" | b"uuid" | b"meta" | b"udta" => discard(&mut out, *atom)?,
            _ => return Err(ERROR), // Includes fragmented movies; no raw fallback.
        }
    }
    single(&children, b"mvhd")?;
    let mut ranges = Vec::new();
    let mut saw_video = false;
    let mut track_count = 0;
    for atom in children {
        match &atom.kind {
            b"mvhd" => dates(&mut out, atom)?,
            b"trak" => {
                track_count += 1;
                if track_count > 32 {
                    return Err(ERROR);
                }
                let mdia = single(&atoms(data, atom.payload())?, b"mdia")?;
                let kind = handler(data, mdia)?;
                if matches!(&kind, b"vide" | b"soun") {
                    saw_video |= &kind == b"vide";
                    ranges.extend(tracks::samples(data, atom, kind)?);
                    if ranges.len() > MAX_CHUNKS {
                        return Err(ERROR);
                    }
                    scrub(&mut out, data, atom, 0)?;
                } else if matches!(&kind, b"meta" | b"mdta" | b"tmcd" | b"hint") {
                    // Merely deleting this header would leave timed GPS or other
                    // identifying samples in mdat. Unclaimed samples are zeroed below.
                    discard(&mut out, atom)?;
                } else {
                    // Do not silently delete subtitles or other authored content
                    // whose layout this cleaner does not yet understand.
                    return Err(ERROR);
                }
            }
            b"udta" | b"meta" | b"uuid" | b"free" | b"iods" => discard(&mut out, atom)?,
            _ => return Err(ERROR),
        }
    }
    if !saw_video || ranges.is_empty() {
        return Err(ERROR);
    }
    ranges.sort_unstable_by_key(|r| r.start);
    let mut previous_end = 0;
    for r in &ranges {
        if r.start < previous_end || !media.iter().any(|m| r.start >= m.body && r.end <= m.end) {
            return Err(ERROR);
        }
        previous_end = r.end;
    }
    // Offsets remain valid because neither headers nor sample data move.
    let mut kept = ranges.iter().peekable();
    for atom in media {
        let mut at = atom.body;
        while kept.peek().is_some_and(|r| r.start < atom.end) {
            let r = kept.next().ok_or(ERROR)?;
            zero(&mut out, at..r.start)?;
            at = r.end;
        }
        zero(&mut out, at..atom.end)?;
    }
    Ok(out)
}

fn scrub(out: &mut [u8], input: &[u8], container: Atom, depth: usize) -> Result<()> {
    if depth > 8 {
        return Err(ERROR);
    }
    for atom in atoms(input, container.payload())? {
        match &atom.kind {
            b"mdia" | b"minf" | b"dinf" | b"stbl" | b"edts" => scrub(out, input, atom, depth + 1)?,
            b"tkhd" | b"mdhd" => dates(out, atom)?,
            b"hdlr" => {
                if atom.end - atom.body < 24 {
                    return Err(ERROR);
                }
                // QuickTime's handler manufacturer and name can identify a device.
                zero(out, atom.body + 12..atom.end)?;
            }
            b"dref" => data_references(out, input, atom)?,
            b"stsd" => tracks::descriptions(out, input, atom)?,
            b"sbgp" | b"sgpd" => {
                if !matches!(bytes(input, atom.body + 4, 4)?, b"roll" | b"prol") {
                    return Err(ERROR);
                }
            }
            b"vmhd" | b"smhd" | b"elst" | b"stts" | b"ctts" | b"cslg" | b"stsc" | b"stsz"
            | b"stco" | b"co64" | b"stss" | b"stps" | b"sdtp" => {}
            b"udta" | b"meta" | b"uuid" | b"free" | b"tref" | b"name" => discard(out, atom)?,
            // Sample groups may contain codec/encryption-dependent data. Reject
            // unknown structures instead of copying opaque personal metadata.
            _ => return Err(ERROR),
        }
    }
    Ok(())
}

fn data_references(out: &mut [u8], input: &[u8], atom: Atom) -> Result<()> {
    let body = bytes(input, atom.body, atom.end - atom.body)?;
    if uint(body, 0, 4)? != 0 || uint(body, 4, 4)? != 1 {
        return Err(ERROR);
    }
    let entries = atoms(input, atom.body + 8..atom.end)?;
    let entry = single(&entries, b"url ")?;
    if entries.len() != 1 || uint(input, entry.body, 4)? != 1 {
        return Err(ERROR);
    }
    // Only a self-contained reference is allowed. Never retain a local path/URL.
    zero(out, entry.body + 4..entry.end)
}

#[cfg(test)]
#[path = "video_metadata/tests.rs"]
mod tests;
