use super::*;

const MP4: &[u8] = include_bytes!("../../../../../test-fixtures/video-metadata.mp4");
const MOV: &[u8] = include_bytes!("../../../../../test-fixtures/video-metadata.mov");
const PRIVATE: &[u8] = b"PrivateCamera GPS:+60.1699+024.9384/";

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut data = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    data.extend_from_slice(kind);
    data.extend_from_slice(body);
    data
}

fn container(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    atom(kind, &children.concat())
}

fn full_table(kind: &[u8; 4], fields: &[u32]) -> Vec<u8> {
    let mut data = vec![0; 4];
    for field in fields {
        data.extend_from_slice(&field.to_be_bytes());
    }
    atom(kind, &data)
}

fn track(offset: usize, length: usize) -> Vec<u8> {
    let mut description = vec![0; 78];
    description[7] = 1; // The one self-contained data reference.
    description[42..55].copy_from_slice(b"PrivateCamera");
    description.extend_from_slice(&atom(b"avcC", b"codec configuration"));
    let mut stsd = vec![0; 4];
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend_from_slice(&atom(b"avc1", &description));
    let stbl = container(
        b"stbl",
        &[
            atom(b"stsd", &stsd),
            full_table(b"stsz", &[0, 1, length as u32]),
            full_table(b"stsc", &[1, 1, 1, 1]),
            full_table(b"stco", &[1, offset as u32]),
        ],
    );
    let mut hdlr = vec![0; 24];
    hdlr[8..12].copy_from_slice(b"vide");
    hdlr.extend_from_slice(PRIVATE);
    let mut header = vec![0; 32];
    header[4..12].fill(0x75); // Original creation and modification timestamps.
    let mut dref = vec![0; 4];
    dref.extend_from_slice(&1u32.to_be_bytes());
    dref.extend_from_slice(&atom(b"url ", &[0, 0, 0, 1]));
    container(
        b"trak",
        &[
            atom(b"tkhd", &header),
            container(
                b"mdia",
                &[
                    atom(b"mdhd", &header),
                    atom(b"hdlr", &hdlr),
                    container(
                        b"minf",
                        &[container(b"dinf", &[atom(b"dref", &dref)]), stbl],
                    ),
                ],
            ),
            atom(b"udta", PRIVATE),
        ],
    )
}

fn synthetic() -> (Vec<u8>, Range<usize>) {
    let ftyp = atom(b"ftyp", b"isom\0\0\0\0isommp42");
    let mut payload = PRIVATE.to_vec();
    let offset = ftyp.len() + 8 + payload.len();
    payload.extend_from_slice(b"video samples");
    let kept = offset..offset + b"video samples".len();
    payload.extend_from_slice(PRIVATE);
    let mut hdlr = vec![0; 24];
    hdlr[8..12].copy_from_slice(b"mdta");
    let metadata_track = container(
        b"trak",
        &[
            container(b"mdia", &[atom(b"hdlr", &hdlr)]),
            atom(b"udta", PRIVATE),
        ],
    );
    let mut mvhd = vec![0; 100];
    mvhd[4..12].fill(0x75);
    let moov = container(
        b"moov",
        &[
            atom(b"mvhd", &mvhd),
            track(offset, kept.len()),
            metadata_track,
            atom(b"udta", PRIVATE),
        ],
    );
    (
        [ftyp, atom(b"mdat", &payload), moov, atom(b"uuid", PRIVATE)].concat(),
        kept,
    )
}

fn assert_private_removed(data: &[u8]) {
    for needle in [b"Private".as_slice(), b"60.1699", b"24.9384", b"2026-10-31"] {
        assert!(
            !data.windows(needle.len()).any(|s| s == needle),
            "retained {needle:?}"
        );
    }
}

#[test]
fn video_metadata_real_mp4_and_mov_keep_encoded_samples_and_remove_recording_details() {
    for data in [MP4, MOV] {
        assert!(data.windows(7).any(|s| s == b"Private"));
        let out = clean(data).unwrap();
        assert_eq!(out.len(), data.len());
        assert_private_removed(&out);
        let movie = single(&atoms(data, 0..data.len()).unwrap(), b"moov").unwrap();
        for track in atoms(data, movie.payload())
            .unwrap()
            .iter()
            .filter(|a| &a.kind == b"trak")
        {
            let mdia = single(&atoms(data, track.payload()).unwrap(), b"mdia").unwrap();
            for r in tracks::samples(data, *track, handler(data, mdia).unwrap()).unwrap() {
                assert_eq!(&out[r.clone()], &data[r]);
            }
        }
        assert_eq!(clean(&out).unwrap(), out);
    }
}

#[test]
fn video_metadata_removes_timed_metadata_tracks_and_zeroes_unclaimed_sample_bytes() {
    let (data, kept) = synthetic();
    let out = clean(&data).unwrap();
    assert_private_removed(&out);
    assert_eq!(&out[kept], b"video samples");
    assert!(!out.windows(4).any(|s| s == b"mdta"));
    assert_eq!(out.len(), data.len());
    assert_eq!(clean(&out).unwrap(), out);
    let movie = single(&atoms(&out, 0..out.len()).unwrap(), b"moov").unwrap();
    let header = single(&atoms(&out, movie.payload()).unwrap(), b"mvhd").unwrap();
    assert_eq!(&out[header.body + 4..header.body + 12], &[0; 8]);
}

#[test]
fn video_metadata_preserves_rotation_matrix_and_timing() {
    let (mut data, _) = synthetic();
    let marker = data.windows(4).position(|s| s == b"tkhd").unwrap();
    // Synthetic header bytes after dates stand in for timing/display fields.
    data[marker + 16..marker + 32].fill(0x13);
    let out = clean(&data).unwrap();
    assert_eq!(&out[marker + 16..marker + 32], &[0x13; 16]);
}

#[test]
fn video_metadata_detects_renamed_movies_and_rejects_unhandled_video_formats() {
    for name in ["upload.tmp", "video.mp4", "video.mov", "audio.m4a"] {
        assert!(detect(&MP4[..32], Path::new(name)).unwrap());
    }
    assert!(!detect(b"ordinary document", Path::new("notes.txt")).unwrap());
    for name in ["clip.mkv", "clip.webm", "clip.avi", "clip.wmv", "clip.ts"] {
        assert!(detect(b"data", Path::new(name)).is_err());
    }
    assert!(detect(b"\x1a\x45\xdf\xa3", Path::new("upload.tmp")).is_err());
}

#[test]
fn video_metadata_preserves_audio_only_recordings() {
    let audio = include_bytes!("../../../../../test-fixtures/voice-message.m4a");
    assert_eq!(clean(audio).unwrap(), audio);
}

#[test]
fn video_metadata_rejects_unhandled_subtitle_tracks_without_dropping_their_content() {
    let (mut data, _) = synthetic();
    let marker = data.windows(4).position(|s| s == b"mdta").unwrap();
    data[marker..marker + 4].copy_from_slice(b"sbtl");
    assert!(clean(&data).is_err());
}

#[test]
fn video_metadata_rejects_invalid_lengths_offsets_and_external_references() {
    let (data, _) = synthetic();
    for kind in [b"stco", b"stsz", b"stsc"] {
        let mut bad = data.clone();
        let marker = bad.windows(4).position(|s| s == kind).unwrap();
        bad[marker + 12..marker + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(clean(&bad).is_err(), "accepted invalid {kind:?}");
    }
    let mut external = data.clone();
    let marker = external.windows(4).position(|s| s == b"url ").unwrap();
    external[marker + 4..marker + 8].fill(0);
    assert!(clean(&external).is_err());
    let mut fragmented = data.clone();
    fragmented.extend_from_slice(&atom(b"moof", b"fragments"));
    assert!(clean(&fragmented).is_err());
    for end in 0..MP4.len() {
        assert!(
            clean(&MP4[..end]).is_err(),
            "accepted truncated movie at {end}"
        );
    }
}

#[test]
fn video_metadata_rejects_overlapping_audio_video_chunks() {
    let (data, kept) = synthetic();
    let top = atoms(&data, 0..data.len()).unwrap();
    let movie = single(&top, b"moov").unwrap();
    let mut children: Vec<_> = atoms(&data, movie.payload())
        .unwrap()
        .iter()
        .map(|a| data[a.start..a.end].to_vec())
        .collect();
    children.push(track(kept.start, kept.len()));
    let out: Vec<u8> = top
        .iter()
        .flat_map(|a| {
            if &a.kind == b"moov" {
                container(b"moov", &children)
            } else {
                data[a.start..a.end].to_vec()
            }
        })
        .collect();
    assert!(clean(&out).is_err());
}
