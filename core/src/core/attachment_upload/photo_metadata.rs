//! Lossless photo cleanup before any bytes enter the attachment store.
//!
//! Keep image/animation data, colour profiles and a newly built orientation-only
//! EXIF record. Never copy camera EXIF, XMP, comments, thumbnails or trailing data.
//! This does not remove information visible in the pixels or embedded in an ICC
//! colour profile. Unhandled photo containers fail closed; ordinary files pass
//! through. No image decompression or additional dependency is needed.

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

pub(super) fn clean(data: &[u8], format: Format) -> Result<Vec<u8>> {
    match format {
        Format::Jpeg => jpeg(data),
        Format::Png => png(data),
        Format::Webp => webp(data),
        Format::Gif => gif(data),
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

fn jpeg(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = b"\xff\xd8".to_vec();
    let mut at = 2;
    let mut saw_scan = false;
    let mut saw_exif = false;
    while at < data.len() {
        if data.get(at) != Some(&0xff) {
            return Err(ERROR);
        }
        while data.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *data.get(at).ok_or(ERROR)?;
        at += 1;
        if marker == 0xd9 {
            if !saw_scan {
                return Err(ERROR);
            }
            out.extend_from_slice(b"\xff\xd9");
            return Ok(out);
        }
        if marker == 0 || marker == 0xd8 || (0xd0..=0xd7).contains(&marker) {
            return Err(ERROR);
        }
        if marker == 0x01 {
            out.extend_from_slice(&[0xff, marker]);
            continue;
        }
        let len = number(data, at, 2, false)?;
        if len < 2 {
            return Err(ERROR);
        }
        let body = slice(data, at + 2, len - 2)?;
        match marker {
            0xe1 if body.starts_with(b"Exif\0\0") => {
                if saw_exif {
                    return Err(ERROR);
                }
                saw_exif = true;
                if let Some(tiff) = orientation(body)? {
                    let mut exif = b"Exif\0\0".to_vec();
                    exif.extend_from_slice(&tiff);
                    jpeg_segment(&mut out, marker, &exif);
                }
            }
            0xe0 if body.starts_with(b"JFIF\0") => {
                let mut jfif = slice(body, 0, 14)?.to_vec();
                jfif.get_mut(12..14).ok_or(ERROR)?.fill(0); // Discard the embedded preview.
                jpeg_segment(&mut out, marker, &jfif);
            }
            0xe2 if body.starts_with(b"ICC_PROFILE\0") => jpeg_segment(&mut out, marker, body),
            0xee if body.starts_with(b"Adobe") => {
                jpeg_segment(&mut out, marker, slice(body, 0, 12)?)
            }
            0xe0..=0xef | 0xfe => {} // All other application records and comments.
            _ => jpeg_segment(&mut out, marker, body),
        }
        at += len;
        if marker == 0xda {
            saw_scan = true;
            let start = at;
            // Scan entropy-coded data, including escaped FF bytes and restart
            // markers. Continue parsing between scans in progressive JPEGs.
            loop {
                let b = *data.get(at).ok_or(ERROR)?;
                if b == 0xff {
                    let next = *data.get(at + 1).ok_or(ERROR)?;
                    if next == 0 || (0xd0..=0xd7).contains(&next) {
                        at += 2;
                        continue;
                    }
                    break;
                }
                at += 1;
            }
            out.extend_from_slice(slice(data, start, at - start)?);
        }
    }
    Err(ERROR)
}

fn jpeg_segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

fn png(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = slice(data, 0, 8)?.to_vec();
    let mut at = 8;
    let mut saw_pixels = false;
    let mut saw_exif = false;
    while at < data.len() {
        let len = number(data, at, 4, false)?;
        if len > data.len() {
            return Err(ERROR);
        }
        let kind = slice(data, at + 4, 4)?;
        let chunk = slice(data, at, len + 12)?;
        let body = slice(chunk, 8, len)?;
        if at == 8 && (kind != b"IHDR" || len != 13) {
            return Err(ERROR);
        }
        if kind == b"eXIf" {
            if saw_exif {
                return Err(ERROR);
            }
            saw_exif = true;
            if let Some(tiff) = orientation(body)? {
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
            out.extend_from_slice(chunk);
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

fn png_chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc = !0u32;
    for byte in kind.iter().chain(body) {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn webp(data: &[u8]) -> Result<Vec<u8>> {
    let end = number(data, 4, 4, true)?.checked_add(8).ok_or(ERROR)?;
    let data = slice(data, 0, end)?;
    let mut out = b"RIFF\0\0\0\0WEBP".to_vec();
    let mut at = 12;
    let mut header = None;
    let mut has_exif = false;
    let mut saw_exif = false;
    let mut saw_pixels = false;
    while at < end {
        let kind = slice(data, at, 4)?;
        let len = number(data, at + 4, 4, true)?;
        if len > data.len() {
            return Err(ERROR);
        }
        let body = slice(data, at + 8, len)?;
        slice(data, at + 8, len + (len & 1))?;
        if kind == b"EXIF" {
            if saw_exif {
                return Err(ERROR);
            }
            saw_exif = true;
            if let Some(tiff) = orientation(body)? {
                riff_chunk(&mut out, kind, &tiff);
                has_exif = true;
            }
        } else if kind == b"VP8X" {
            if header.is_some() || at != 12 || len != 10 {
                return Err(ERROR);
            }
            header = Some(out.len() + 8);
            riff_chunk(&mut out, kind, body);
        } else if kind == b"ANMF" {
            let mut frame = slice(body, 0, 16)?.to_vec();
            let mut pos = 16;
            while pos < body.len() {
                let tag = slice(body, pos, 4)?;
                let size = number(body, pos + 4, 4, true)?;
                if size > body.len() {
                    return Err(ERROR);
                }
                let payload = slice(body, pos + 8, size + (size & 1))?;
                if matches!(tag, b"ALPH" | b"VP8 " | b"VP8L") {
                    riff_chunk(&mut frame, tag, slice(payload, 0, size)?);
                }
                pos += 8 + size + (size & 1);
            }
            riff_chunk(&mut out, kind, &frame);
            saw_pixels = true;
        } else if matches!(kind, b"VP8 " | b"VP8L" | b"ALPH" | b"ICCP" | b"ANIM") {
            riff_chunk(&mut out, kind, body);
            saw_pixels |= matches!(kind, b"VP8 " | b"VP8L");
        }
        at += 8 + len + (len & 1);
    }
    if !saw_pixels || (has_exif && header.is_none()) {
        return Err(ERROR);
    }
    if let Some(pos) = header {
        let flags = out.get_mut(pos).ok_or(ERROR)?;
        *flags &= !(0x08 | 0x04); // Recompute EXIF and clear XMP feature bits.
        if has_exif {
            *flags |= 0x08;
        }
    }
    let len = (out.len() - 8) as u32;
    out.get_mut(4..8)
        .ok_or(ERROR)?
        .copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

fn riff_chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() & 1 != 0 {
        out.push(0);
    }
}

fn gif(data: &[u8]) -> Result<Vec<u8>> {
    let mut at = 13;
    let flags = *data.get(10).ok_or(ERROR)?;
    if flags & 0x80 != 0 {
        at += 3 * (1 << ((flags & 7) + 1));
    }
    let mut out = slice(data, 0, at)?.to_vec();
    let mut saw_pixels = false;
    loop {
        let start = at;
        let marker = *data.get(at).ok_or(ERROR)?;
        at += 1;
        match marker {
            0x3b if saw_pixels => {
                out.push(marker);
                return Ok(out);
            }
            0x2c => {
                let flags = *slice(data, at, 9)?.get(8).ok_or(ERROR)?;
                at += 9;
                if flags & 0x80 != 0 {
                    at += 3 * (1 << ((flags & 7) + 1));
                }
                slice(data, at, 1)?; // LZW minimum code size.
                at = gif_blocks_end(data, at + 1)?;
                out.extend_from_slice(slice(data, start, at - start)?);
                saw_pixels = true;
            }
            0x21 => {
                let label = *data.get(at).ok_or(ERROR)?;
                at += 1;
                let blocks = at;
                at = gif_blocks_end(data, at)?;
                if label == 0xf9 {
                    if at - blocks != 6 || data.get(blocks) != Some(&4) {
                        return Err(ERROR);
                    }
                    out.extend_from_slice(slice(data, start, at - start)?);
                } else if label == 0xff
                    && at - blocks == 17
                    && data.get(blocks) == Some(&11)
                    && matches!(
                        slice(data, blocks + 1, 11)?,
                        b"NETSCAPE2.0" | b"ANIMEXTS1.0"
                    )
                    && slice(data, blocks + 12, 2)? == [3, 1]
                {
                    out.extend_from_slice(slice(data, start, at - start)?); // Animation repeat count only.
                } else if label == 0x01 {
                    return Err(ERROR); // Plain-text rendering is not a photo.
                }
            }
            _ => return Err(ERROR),
        }
    }
}

fn gif_blocks_end(data: &[u8], mut at: usize) -> Result<usize> {
    loop {
        let len = *data.get(at).ok_or(ERROR)? as usize;
        at += 1;
        slice(data, at, len)?;
        at += len;
        if len == 0 {
            return Ok(at);
        }
    }
}

#[cfg(test)]
#[path = "photo_metadata_tests.rs"]
pub(super) mod tests;
