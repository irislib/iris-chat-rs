//! Lossless photo cleanup before any bytes enter the attachment store.
//!
//! Keep image/animation data, colour profiles and a newly built orientation-only
//! EXIF record. Never copy camera EXIF, XMP, comments, thumbnails or trailing data.
//! This does not remove information visible in the pixels or embedded in an ICC
//! colour profile. Unhandled photo containers fail closed; ordinary files pass
//! through. No image decompression or additional dependency is needed. Pixel
//! data is streamed from the file; only kept records are held in memory.

use super::clean_stream::{Plan, Source};
use std::path::Path;

pub(super) const ERROR: &str = "Could not remove photo details. Try a JPEG or PNG copy.";
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy)]
pub(super) enum Format {
    Jpeg,
    Png,
    Webp,
    Gif,
}

pub(super) fn detect(data: &[u8], path: &Path) -> Result<Option<Format>> {
    if data.starts_with(b"\xff\xd8") {
        return Ok(Some(Format::Jpeg));
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(Some(Format::Png));
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return Ok(Some(Format::Webp));
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Ok(Some(Format::Gif));
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // HEIF/AVIF and TIFF require a decoder or an item-aware rewrite. Do not
    // silently send the original while claiming that its photo details are gone.
    let image_container = data.starts_with(b"II\x2a\0")
        || data.starts_with(b"MM\0\x2a")
        || data.starts_with(b"BM")
        || (data.get(4..8) == Some(b"ftyp")
            && data.get(8..12).is_some_and(|brand| {
                matches!(
                    brand,
                    b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
                )
            }));
    if image_container
        || matches!(
            ext.as_str(),
            "jpg"
                | "jpeg"
                | "jpe"
                | "jfif"
                | "png"
                | "apng"
                | "webp"
                | "gif"
                | "heic"
                | "heif"
                | "avif"
                | "tif"
                | "tiff"
                | "bmp"
                | "dng"
        )
    {
        return Err(ERROR);
    }
    Ok(None)
}

pub(super) fn plan(source: &Source, format: Format) -> Result<Plan> {
    match format {
        Format::Jpeg => jpeg(source),
        Format::Png => png(source),
        Format::Webp => webp(source),
        Format::Gif => gif(source),
    }
}

#[cfg(test)]
pub(super) fn clean(data: &[u8], format: Format) -> Result<Vec<u8>> {
    super::clean_stream::run(data, |source| plan(source, format))
}

fn read(source: &Source, at: usize, len: usize) -> Result<Vec<u8>> {
    source.read(at, len).ok_or(ERROR)
}

fn byte(source: &Source, at: usize) -> Result<u8> {
    source.byte(at).ok_or(ERROR)
}

fn read_number(source: &Source, at: usize, len: usize, little: bool) -> Result<usize> {
    number(&read(source, at, len)?, 0, len, little)
}

/// Check that `len` bytes at `at` end within `end` without reading them.
fn within(at: usize, len: usize, end: usize) -> Result<()> {
    match at.checked_add(len) {
        Some(stop) if stop <= end => Ok(()),
        _ => Err(ERROR),
    }
}

fn slice(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    data.get(at..at.checked_add(len).ok_or(ERROR)?).ok_or(ERROR)
}

fn number(data: &[u8], at: usize, len: usize, little: bool) -> Result<usize> {
    let bytes = slice(data, at, len)?;
    Ok(if little {
        bytes.iter().rev().fold(0, |n, b| (n << 8) | *b as usize)
    } else {
        bytes.iter().fold(0, |n, b| (n << 8) | *b as usize)
    })
}

// Read only IFD0's orientation; never follow GPS, EXIF or thumbnail pointers.
fn orientation(exif: &[u8]) -> Result<Option<Vec<u8>>> {
    let exif = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let little = match exif.get(..4) {
        Some(b"II\x2a\0") => true,
        Some(b"MM\0\x2a") => false,
        _ => return Err(ERROR),
    };
    let offset = number(exif, 4, 4, little)?;
    if offset == 0 {
        return Ok(None);
    }
    let count = number(exif, offset, 2, little)?;
    let entries = slice(exif, offset + 2, count * 12)?;
    let mut value = None;
    for entry in entries.as_chunks::<12>().0 {
        if number(entry, 0, 2, little)? != 0x112 {
            continue;
        }
        if value.is_some() || number(entry, 2, 2, little)? != 3 || number(entry, 4, 4, little)? != 1
        {
            return Err(ERROR);
        }
        let n = number(entry, 8, 2, little)?;
        if !(1..=8).contains(&n) {
            return Err(ERROR);
        }
        value = Some(n as u8);
    }
    Ok(value.map(|n| {
        // Little-endian TIFF, one SHORT orientation entry, no next IFD.
        let mut out = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        out.extend_from_slice(&[n, 0, 0, 0, 0, 0, 0, 0]);
        out
    }))
}

fn jpeg(source: &Source) -> Result<Plan> {
    let mut out = Plan::default();
    out.push(b"\xff\xd8");
    let mut at = 2;
    let mut saw_scan = false;
    let mut saw_exif = false;
    while at < source.len() {
        if byte(source, at)? != 0xff {
            return Err(ERROR);
        }
        while source.byte(at) == Some(0xff) {
            at += 1;
        }
        let marker = byte(source, at)?;
        at += 1;
        if marker == 0xd9 {
            if !saw_scan {
                return Err(ERROR);
            }
            out.push(b"\xff\xd9");
            return Ok(out);
        }
        if marker == 0 || marker == 0xd8 || (0xd0..=0xd7).contains(&marker) {
            return Err(ERROR);
        }
        if marker == 0x01 {
            out.push(&[0xff, marker]);
            continue;
        }
        let len = read_number(source, at, 2, false)?;
        if len < 2 {
            return Err(ERROR);
        }
        // Segment lengths are 16-bit, so a kept record stays small.
        let body = read(source, at + 2, len - 2)?;
        match marker {
            0xe1 if body.starts_with(b"Exif\0\0") => {
                if saw_exif {
                    return Err(ERROR);
                }
                saw_exif = true;
                if let Some(tiff) = orientation(&body)? {
                    let mut exif = b"Exif\0\0".to_vec();
                    exif.extend_from_slice(&tiff);
                    jpeg_segment(&mut out, marker, &exif);
                }
            }
            0xe0 if body.starts_with(b"JFIF\0") => {
                let mut jfif = slice(&body, 0, 14)?.to_vec();
                jfif.get_mut(12..14).ok_or(ERROR)?.fill(0); // Discard the embedded preview.
                jpeg_segment(&mut out, marker, &jfif);
            }
            0xe2 if body.starts_with(b"ICC_PROFILE\0") => jpeg_segment(&mut out, marker, &body),
            0xee if body.starts_with(b"Adobe") => {
                jpeg_segment(&mut out, marker, slice(&body, 0, 12)?)
            }
            0xe0..=0xef | 0xfe => {} // All other application records and comments.
            _ => jpeg_segment(&mut out, marker, &body),
        }
        at += len;
        if marker == 0xda {
            saw_scan = true;
            let start = at;
            at = jpeg_scan_end(source, at)?;
            out.copy(start, at - start);
        }
    }
    Err(ERROR)
}

/// Find the marker ending entropy-coded data, skipping escaped FF bytes and
/// restart markers. Parsing continues between scans in progressive JPEGs.
fn jpeg_scan_end(source: &Source, mut at: usize) -> Result<usize> {
    const WINDOW: usize = 64 * 1024;
    loop {
        if at >= source.len() {
            return Err(ERROR);
        }
        let window = read(source, at, WINDOW.min(source.len() - at))?;
        let mut i = 0;
        while let Some(found) = window
            .get(i..)
            .and_then(|w| w.iter().position(|b| *b == 0xff))
        {
            i += found;
            let next = match window.get(i + 1) {
                Some(next) => *next,
                None => byte(source, at + i + 1)?,
            };
            if next != 0 && !(0xd0..=0xd7).contains(&next) {
                return Ok(at + i);
            }
            i += 2;
        }
        at += i.max(window.len());
    }
}

fn jpeg_segment(out: &mut Plan, marker: u8, body: &[u8]) {
    out.push(&[0xff, marker]);
    out.push(&((body.len() + 2) as u16).to_be_bytes());
    out.push(body);
}

fn png(source: &Source) -> Result<Plan> {
    let mut out = Plan::default();
    within(0, 8, source.len())?;
    out.copy(0, 8);
    let mut at = 8;
    let mut saw_pixels = false;
    let mut saw_exif = false;
    while at < source.len() {
        let len = read_number(source, at, 4, false)?;
        if len > source.len() {
            return Err(ERROR);
        }
        let kind = read(source, at + 4, 4)?;
        let kind = kind.as_slice();
        within(at, len + 12, source.len())?;
        if at == 8 && (kind != b"IHDR" || len != 13) {
            return Err(ERROR);
        }
        if kind == b"eXIf" {
            if saw_exif {
                return Err(ERROR);
            }
            saw_exif = true;
            if let Some(tiff) = orientation(&read(source, at + 8, len)?)? {
                png_chunk(&mut out, kind, &tiff);
            }
        } else if matches!(
            kind,
            b"IHDR"
                | b"PLTE"
                | b"IDAT"
                | b"IEND"
                | b"tRNS"
                | b"cHRM"
                | b"gAMA"
                | b"iCCP"
                | b"sRGB"
                | b"sBIT"
                | b"bKGD"
                | b"pHYs"
                | b"acTL"
                | b"fcTL"
                | b"fdAT"
                | b"cICP"
                | b"mDCv"
                | b"cLLi"
        ) {
            out.copy(at, len + 12);
        } else if kind.first().ok_or(ERROR)? & 0x20 == 0 {
            return Err(ERROR); // Unknown critical chunk: cannot safely rewrite.
        }
        saw_pixels |= kind == b"IDAT";
        if kind == b"IEND" {
            return if len == 0 && saw_pixels {
                Ok(out)
            } else {
                Err(ERROR)
            };
        }
        at += len + 12;
    }
    Err(ERROR)
}

fn png_chunk(out: &mut Plan, kind: &[u8], body: &[u8]) {
    out.push(&(body.len() as u32).to_be_bytes());
    out.push(kind);
    out.push(body);
    let mut crc = !0u32;
    for byte in kind.iter().chain(body) {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    out.push(&(!crc).to_be_bytes());
}

fn webp(source: &Source) -> Result<Plan> {
    let end = read_number(source, 4, 4, true)?
        .checked_add(8)
        .ok_or(ERROR)?;
    within(0, end, source.len())?;
    let mut out = Plan::default();
    out.push(b"RIFF\0\0\0\0WEBP");
    let mut at = 12;
    let mut header = None;
    let mut has_exif = false;
    let mut saw_exif = false;
    let mut saw_pixels = false;
    while at < end {
        within(at, 8, end)?;
        let kind = read(source, at, 4)?;
        let kind = kind.as_slice();
        let len = read_number(source, at + 4, 4, true)?;
        if len > end {
            return Err(ERROR);
        }
        within(at + 8, len + (len & 1), end)?;
        let body = at + 8;
        if kind == b"EXIF" {
            if saw_exif {
                return Err(ERROR);
            }
            saw_exif = true;
            if let Some(tiff) = orientation(&read(source, body, len)?)? {
                riff_chunk(&mut out, kind, &tiff);
                has_exif = true;
            }
        } else if kind == b"VP8X" {
            if header.is_some() || at != 12 || len != 10 {
                return Err(ERROR);
            }
            header = Some(out.len() + 8);
            riff_chunk(&mut out, kind, &read(source, body, len)?);
        } else if kind == b"ANMF" {
            let frame = read(source, body, 16)?;
            within(0, 16, len)?;
            let mut kept = Vec::new();
            let mut frame_len = frame.len();
            let mut pos = 16;
            while pos < len {
                within(pos, 8, len)?;
                let tag = read(source, body + pos, 4)?;
                let size = read_number(source, body + pos + 4, 4, true)?;
                if size > len {
                    return Err(ERROR);
                }
                within(pos + 8, size + (size & 1), len)?;
                if matches!(tag.as_slice(), b"ALPH" | b"VP8 " | b"VP8L") {
                    kept.push((tag, body + pos + 8, size));
                    frame_len += 8 + size + (size & 1);
                }
                pos += 8 + size + (size & 1);
            }
            out.push(kind);
            out.push(&u32::try_from(frame_len).map_err(|_| ERROR)?.to_le_bytes());
            out.push(&frame);
            for (tag, at, size) in kept {
                copied_riff_chunk(&mut out, &tag, at, size);
            }
            saw_pixels = true;
        } else if matches!(kind, b"VP8 " | b"VP8L" | b"ALPH" | b"ICCP" | b"ANIM") {
            copied_riff_chunk(&mut out, kind, body, len);
            saw_pixels |= matches!(kind, b"VP8 " | b"VP8L");
        }
        at += 8 + len + (len & 1);
    }
    if !saw_pixels || (has_exif && header.is_none()) {
        return Err(ERROR);
    }
    if let Some(pos) = header {
        let flags = out
            .literal_mut(pos, 1)
            .and_then(|bytes| bytes.first_mut())
            .ok_or(ERROR)?;
        *flags &= !(0x08 | 0x04); // Recompute EXIF and clear XMP feature bits.
        if has_exif {
            *flags |= 0x08;
        }
    }
    let len = u32::try_from(out.len() - 8).map_err(|_| ERROR)?;
    out.literal_mut(4, 4)
        .ok_or(ERROR)?
        .copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

fn riff_chunk(out: &mut Plan, kind: &[u8], body: &[u8]) {
    out.push(kind);
    out.push(&(body.len() as u32).to_le_bytes());
    out.push(body);
    if body.len() & 1 != 0 {
        out.push(&[0]);
    }
}

fn copied_riff_chunk(out: &mut Plan, kind: &[u8], at: usize, len: usize) {
    out.push(kind);
    out.push(&(len as u32).to_le_bytes());
    out.copy(at, len);
    if len & 1 != 0 {
        out.push(&[0]);
    }
}

fn gif(source: &Source) -> Result<Plan> {
    let mut at = 13;
    let flags = byte(source, 10)?;
    if flags & 0x80 != 0 {
        at += 3 * (1 << ((flags & 7) + 1));
    }
    within(0, at, source.len())?;
    let mut out = Plan::default();
    out.copy(0, at);
    let mut saw_pixels = false;
    loop {
        let start = at;
        let marker = byte(source, at)?;
        at += 1;
        match marker {
            0x3b if saw_pixels => {
                out.push(&[marker]);
                return Ok(out);
            }
            0x2c => {
                let flags = byte(source, at + 8)?;
                at += 9;
                if flags & 0x80 != 0 {
                    at += 3 * (1 << ((flags & 7) + 1));
                }
                byte(source, at)?; // LZW minimum code size.
                at = gif_blocks_end(source, at + 1)?;
                out.copy(start, at - start);
                saw_pixels = true;
            }
            0x21 => {
                let label = byte(source, at)?;
                at += 1;
                let blocks = at;
                at = gif_blocks_end(source, at)?;
                if label == 0xf9 {
                    if at - blocks != 6 || byte(source, blocks)? != 4 {
                        return Err(ERROR);
                    }
                    out.copy(start, at - start);
                } else if label == 0xff
                    && at - blocks == 17
                    && byte(source, blocks)? == 11
                    && matches!(
                        read(source, blocks + 1, 11)?.as_slice(),
                        b"NETSCAPE2.0" | b"ANIMEXTS1.0"
                    )
                    && read(source, blocks + 12, 2)? == [3, 1]
                {
                    out.copy(start, at - start); // Animation repeat count only.
                } else if label == 0x01 {
                    return Err(ERROR); // Plain-text rendering is not a photo.
                }
            }
            _ => return Err(ERROR),
        }
    }
}

fn gif_blocks_end(source: &Source, mut at: usize) -> Result<usize> {
    loop {
        let len = byte(source, at)? as usize;
        at += 1;
        within(at, len, source.len())?;
        at += len;
        if len == 0 {
            return Ok(at);
        }
    }
}

#[cfg(test)]
#[path = "photo_metadata_tests.rs"]
pub(super) mod tests;
