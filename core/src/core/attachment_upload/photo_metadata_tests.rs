use super::*;

const PRIVATE: &[u8] = b"GPS:60.1699,24.9384 CameraSerial:12345 Taken:2026-10-31 Author:Alice";
const PNG: &[u8] =
    include_bytes!("../../../../android/app/src/main/res/drawable-nodpi/iris_logo.png");

fn exif(n: u8, little: bool) -> Vec<u8> {
    // IFD0 includes orientation and a GPS pointer to identifying test data.
    let mut out = if little {
        b"II\x2a\0\x08\0\0\0\x02\0\x12\x01\x03\0\x01\0\0\0\0\0\0\0\x25\x88\x04\0\x01\0\0\0\x26\0\0\0\0\0\0\0".to_vec()
    } else {
        b"MM\0\x2a\0\0\0\x08\0\x02\x01\x12\0\x03\0\0\0\x01\0\0\0\0\x88\x25\0\x04\0\0\0\x01\0\0\0\x26\0\0\0\0".to_vec()
    };
    out[if little { 18 } else { 19 }] = n;
    out.extend_from_slice(PRIVATE);
    out
}

fn assert_private_removed(data: &[u8]) {
    for text in [b"GPS".as_slice(), b"CameraSerial", b"Taken", b"Author"] {
        assert!(!data.windows(text.len()).any(|part| part == text));
    }
}

fn jpeg_fixture() -> Vec<u8> {
    let mut out = b"\xff\xd8".to_vec();
    jpeg_segment(&mut out, 0xe0, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0");
    jpeg_segment(&mut out, 0xc0, b"frame");
    jpeg_segment(&mut out, 0xda, b"scan-one");
    out.extend_from_slice(b"pixels\xff\0stuffed\xff\xd0restart");
    // A comment between scans must not survive a progressive JPEG upload.
    jpeg_segment(&mut out, 0xfe, PRIVATE);
    jpeg_segment(&mut out, 0xda, b"scan-two");
    out.extend_from_slice(b"more-pixels\xff\xd9");
    out
}

#[test]
fn photo_metadata_jpeg_removes_all_private_records_and_trailers_without_reencoding() {
    for little in [false, true] {
        for n in 1..=8 {
            let mut data = b"\xff\xd8".to_vec();
            let mut metadata = b"Exif\0\0".to_vec();
            metadata.extend_from_slice(&exif(n, little));
            jpeg_segment(&mut data, 0xe1, &metadata);
            for marker in [0xe1, 0xe3, 0xed, 0xfe] {
                jpeg_segment(&mut data, marker, PRIVATE);
            }
            data.extend_from_slice(&jpeg_fixture()[2..]);
            data.extend_from_slice(PRIVATE);
            let out = clean(&data, Format::Jpeg).unwrap();
            assert_private_removed(&out);
            assert!(out.windows(6).any(|s| s == b"pixels"));
            let exif_at = out.windows(6).position(|s| s == b"Exif\0\0").unwrap();
            assert_eq!(out[exif_at + 6 + 18], n);
            assert_eq!(clean(&out, Format::Jpeg).unwrap(), out);
        }
    }
}

#[test]
fn photo_metadata_jpeg_preserves_colour_and_drops_embedded_thumbnail() {
    let mut data = b"\xff\xd8".to_vec();
    let mut jfif = b"JFIF\0\x01\x02\0\0\x01\0\x01\x01\x01".to_vec();
    jfif.extend_from_slice(PRIVATE);
    jpeg_segment(&mut data, 0xe0, &jfif);
    jpeg_segment(&mut data, 0xe2, b"ICC_PROFILE\0\x01\x01colour");
    jpeg_segment(&mut data, 0xee, b"Adobe\0\x64\0\0\0\0\x01");
    data.extend_from_slice(&jpeg_fixture()[2..]);
    let out = jpeg(&data).unwrap();
    assert_private_removed(&out);
    assert!(out.windows(6).any(|s| s == b"colour"));
    assert!(out.windows(5).any(|s| s == b"Adobe"));
}

pub(in super::super) fn private_png() -> Vec<u8> {
    let mut out = PNG[..33].to_vec();
    for kind in [b"tEXt", b"iTXt", b"zTXt", b"tIME", b"caBX", b"prIv"] {
        png_chunk(&mut out, kind, PRIVATE);
    }
    png_chunk(&mut out, b"eXIf", &exif(6, true));
    out.extend_from_slice(&PNG[33..]);
    out.extend_from_slice(PRIVATE);
    out
}

#[test]
fn photo_metadata_png_removes_text_exif_time_and_unknown_chunks_but_keeps_pixels() {
    let out = clean(&private_png(), Format::Png).unwrap();
    assert_private_removed(&out);
    let expected = png(PNG).unwrap();
    // Remove the orientation-only chunk and compare the entire remaining file.
    let mut without_orientation = out[..33].to_vec();
    without_orientation.extend_from_slice(&out[33 + 26 + 12..]);
    assert_eq!(without_orientation, expected);
    assert_eq!(png(&out).unwrap(), out);
}

#[test]
fn photo_metadata_png_preserves_animation_and_colour_chunks() {
    let mut data = PNG[..33].to_vec();
    for kind in [b"acTL", b"fcTL", b"fdAT", b"iCCP", b"cICP"] {
        png_chunk(&mut data, kind, b"rendering");
    }
    data.extend_from_slice(&PNG[33..]);
    let out = png(&data).unwrap();
    for kind in [b"acTL", b"fcTL", b"fdAT", b"iCCP", b"cICP"] {
        assert!(out.windows(4).any(|s| s == kind));
    }
}

fn webp_fixture() -> Vec<u8> {
    let mut data = b"RIFF\0\0\0\0WEBP".to_vec();
    riff_chunk(&mut data, b"VP8X", &[0x0e, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    riff_chunk(&mut data, b"ANIM", &[0, 0, 0, 0, 0, 0]);
    let mut frame = vec![0; 16];
    riff_chunk(&mut frame, b"VP8L", b"pixels");
    riff_chunk(&mut frame, b"XMP ", PRIVATE);
    riff_chunk(&mut data, b"ANMF", &frame);
    riff_chunk(&mut data, b"EXIF", &exif(8, false));
    riff_chunk(&mut data, b"XMP ", PRIVATE);
    riff_chunk(&mut data, b"C2PA", PRIVATE);
    let len = data.len() as u32 - 8;
    data[4..8].copy_from_slice(&len.to_le_bytes());
    data.extend_from_slice(PRIVATE);
    data
}

#[test]
fn photo_metadata_webp_preserves_animation_orientation_and_correct_container_sizes() {
    let out = clean(&webp_fixture(), Format::Webp).unwrap();
    assert_private_removed(&out);
    assert_eq!(number(&out, 4, 4, true).unwrap(), out.len() - 8);
    assert_eq!(out[20], 0x0a); // Animation + orientation-only EXIF, no XMP.
    assert!(out.windows(6).any(|s| s == b"pixels"));
    let at = out.windows(4).position(|s| s == b"EXIF").unwrap();
    assert_eq!(out[at + 8 + 18], 8);
    assert_eq!(webp(&out).unwrap(), out);
}

fn gif_fixture() -> Vec<u8> {
    let mut out = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff".to_vec();
    out.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\0\0\0");
    out.extend_from_slice(&[0x21, 0xfe, PRIVATE.len() as u8]);
    out.extend_from_slice(PRIVATE);
    out.push(0);
    out.extend_from_slice(
        b"\x21\xf9\x04\x01\x0a\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0\x3b",
    );
    out.extend_from_slice(PRIVATE);
    out
}

#[test]
fn photo_metadata_gif_preserves_pixels_transparency_delay_and_loop_count() {
    let out = clean(&gif_fixture(), Format::Gif).unwrap();
    assert_private_removed(&out);
    assert!(out.windows(11).any(|s| s == b"NETSCAPE2.0"));
    assert!(out.windows(4).any(|s| s == b"\x01\x0a\0\0"));
    assert_eq!(gif(&out).unwrap(), out);
}

#[test]
fn photo_metadata_detects_content_even_with_an_unrelated_filename() {
    for (data, format) in [
        (jpeg_fixture(), Format::Jpeg),
        (PNG.to_vec(), Format::Png),
        (webp_fixture(), Format::Webp),
        (gif_fixture(), Format::Gif),
    ] {
        let detected = detect(&data[..data.len().min(32)], Path::new("upload.tmp"))
            .unwrap()
            .unwrap();
        assert_eq!(
            clean(&data, detected).unwrap(),
            clean(&data, format).unwrap()
        );
    }
    assert!(detect(b"ordinary document", Path::new("notes.txt"))
        .unwrap()
        .is_none());
    for name in [
        "bad.jpg",
        "bad.PNG",
        "photo.heic",
        "photo.tiff",
        "photo.avif",
    ] {
        assert!(detect(b"bad", Path::new(name)).is_err());
    }
    assert!(detect(b"\0\0\0\x18ftypheic", Path::new("upload.tmp")).is_err());
}

#[test]
fn photo_metadata_rejects_truncation_bad_lengths_and_malformed_exif() {
    for (data, format) in [
        (jpeg_fixture(), Format::Jpeg),
        (PNG.to_vec(), Format::Png),
        (webp_fixture(), Format::Webp),
        (gif_fixture(), Format::Gif),
    ] {
        let complete_end = if matches!(format, Format::Webp | Format::Gif) {
            data.len() - PRIVATE.len()
        } else {
            data.len()
        };
        for end in 0..complete_end {
            assert!(
                clean(&data[..end], format).is_err(),
                "accepted truncation at {end}"
            );
        }
    }
    let mut bad_png = PNG.to_vec();
    bad_png[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(png(&bad_png).is_err());
    assert!(jpeg(b"\xff\xd8\xff\xe1\0\x01").is_err());
    assert!(orientation(b"not exif").is_err());
    assert!(orientation(&exif(9, true)).is_err());
}
