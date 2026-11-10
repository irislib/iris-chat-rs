use super::*;

pub(super) fn samples(data: &[u8], track: Atom, kind: [u8; 4]) -> Result<Vec<Range<usize>>> {
    let mdia = single(&atoms(data, track.payload())?, b"mdia")?;
    let minf = single(&atoms(data, mdia.payload())?, b"minf")?;
    let stbl = single(&atoms(data, minf.payload())?, b"stbl")?;
    let tables = atoms(data, stbl.payload())?;
    let stsd = single(&tables, b"stsd")?;
    let entries = description_entries(data, stsd)?;
    let entry = entries.first().ok_or(ERROR)?;
    if (&kind == b"vide") != is_video(&entry.kind) {
        return Err(ERROR);
    }
    let stsz = single(&tables, b"stsz")?;
    let sizes = bytes(data, stsz.body, stsz.end - stsz.body)?;
    if uint(sizes, 0, 4)? != 0 {
        return Err(ERROR);
    }
    let fixed_size = uint(sizes, 4, 4)?;
    let sample_count = uint(sizes, 8, 4)?;
    if sample_count == 0 || sample_count > data.len() {
        return Err(ERROR);
    }
    if sizes.len()
        != 12
            + if fixed_size == 0 {
                sample_count.checked_mul(4).ok_or(ERROR)?
            } else {
                0
            }
    {
        return Err(ERROR);
    }

    let offset_atoms: Vec<_> = tables
        .iter()
        .filter(|a| matches!(&a.kind, b"stco" | b"co64"))
        .collect();
    if offset_atoms.len() != 1 {
        return Err(ERROR);
    }
    let offsets_atom = **offset_atoms.first().ok_or(ERROR)?;
    let width = if &offsets_atom.kind == b"stco" { 4 } else { 8 };
    let offsets = table(data, offsets_atom, width)?;
    let chunk_count = offsets.len() / width;
    if chunk_count == 0 || chunk_count > MAX_CHUNKS {
        return Err(ERROR);
    }
    let mappings = table(data, single(&tables, b"stsc")?, 12)?;
    let mut map = mappings.as_chunks::<12>().0.iter().peekable();
    let mut current = *map.next().ok_or(ERROR)?;
    if uint(&current, 0, 4)? != 1 {
        return Err(ERROR);
    }
    let mut consumed = 0;
    let mut ranges = Vec::with_capacity(chunk_count);
    for (i, offset) in offsets.chunks(width).enumerate() {
        let chunk = i + 1;
        if map
            .peek()
            .is_some_and(|m| uint(m.as_slice(), 0, 4).ok() == Some(chunk))
        {
            current = *map.next().ok_or(ERROR)?;
        }
        if map
            .peek()
            .is_some_and(|m| uint(m.as_slice(), 0, 4).is_ok_and(|n| n <= chunk || n > chunk_count))
        {
            return Err(ERROR);
        }
        let count = uint(&current, 4, 4)?;
        if count == 0 || uint(&current, 8, 4)? != 1 || count > sample_count - consumed {
            return Err(ERROR);
        }
        let mut len: usize = 0;
        if fixed_size != 0 {
            len = fixed_size.checked_mul(count).ok_or(ERROR)?;
        } else {
            for index in consumed..consumed + count {
                len = len
                    .checked_add(uint(sizes, 12 + index * 4, 4)?)
                    .ok_or(ERROR)?;
            }
        }
        consumed += count;
        let start = uint(offset, 0, width)?;
        if len > 0 {
            ranges.push(start..start.checked_add(len).ok_or(ERROR)?);
        }
    }
    if consumed != sample_count || map.next().is_some() {
        return Err(ERROR);
    }
    Ok(ranges)
}

fn table(data: &[u8], atom: Atom, width: usize) -> Result<&[u8]> {
    let body = bytes(data, atom.body, atom.end - atom.body)?;
    if uint(body, 0, 4)? != 0 {
        return Err(ERROR);
    }
    let count = uint(body, 4, 4)?;
    let len = count.checked_mul(width).ok_or(ERROR)?;
    if len.checked_add(8) != Some(body.len()) {
        return Err(ERROR);
    }
    bytes(body, 8, len)
}

fn description_entries(data: &[u8], atom: Atom) -> Result<Vec<Atom>> {
    let body = bytes(data, atom.body, atom.end - atom.body)?;
    if uint(body, 0, 4)? != 0 || uint(body, 4, 4)? != 1 {
        return Err(ERROR);
    }
    let entries = atoms(data, atom.body + 8..atom.end)?;
    if entries.len() != 1 {
        return Err(ERROR);
    }
    Ok(entries)
}

fn is_video(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"mp4v" | b"av01" | b"vp09" | b"jpeg"
    )
}

pub(super) fn descriptions(out: &mut [u8], data: &[u8], atom: Atom) -> Result<()> {
    for entry in description_entries(data, atom)? {
        let body = bytes(data, entry.body, entry.end - entry.body)?;
        if uint(body, 6, 2)? != 1 {
            return Err(ERROR);
        }
        let header = if is_video(&entry.kind) {
            bytes(body, 0, 78)?;
            zero(out, entry.body + 42..entry.body + 74)?; // Compressor name.
            78
        } else if matches!(
            &entry.kind,
            b"mp4a" | b"ac-3" | b"ec-3" | b"alac" | b"Opus" | b"fLaC" | b"lpcm" | b"sowt" | b"twos"
        ) {
            match uint(body, 8, 2)? {
                0 => 28,
                1 => 44,
                2 => 64,
                _ => return Err(ERROR),
            }
        } else {
            return Err(ERROR);
        }; // Includes encrypted samples.
        bytes(body, 0, header)?;
        zero(out, entry.body + 12..entry.body + 16)?; // QuickTime vendor code.
        extensions(out, data, entry.body + header..entry.end, 0)?;
    }
    Ok(())
}

fn extensions(out: &mut [u8], data: &[u8], range: Range<usize>, depth: usize) -> Result<()> {
    if depth > 4 {
        return Err(ERROR);
    }
    for atom in atoms(data, range)? {
        match &atom.kind {
            b"avcC" | b"hvcC" | b"av1C" | b"vpcC" | b"esds" | b"dac3" | b"dec3" | b"alac"
            | b"dOps" | b"dfLa" | b"colr" | b"clap" | b"pasp" | b"fiel" | b"gama" | b"btrt"
            | b"clli" | b"mdcv" | b"frma" | b"enda" | b"chan" => {}
            b"mp4a" | b"\0\0\0\0"
                if data
                    .get(atom.payload())
                    .is_some_and(|b| b.iter().all(|n| *n == 0)) => {}
            b"wave" => extensions(out, data, atom.payload(), depth + 1)?,
            b"udta" | b"meta" | b"uuid" | b"free" => discard(out, atom)?,
            _ => return Err(ERROR),
        }
    }
    Ok(())
}
