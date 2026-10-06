// Standalone runner: python3 scripts/bench_video_conversion.py
// Baseline is the same production video.rs with only its RGB conversion replaced.
use super::*;
use std::{hint::black_box, time::Instant};

fn frames(width: usize, height: usize, pattern: usize) -> Vec<Vec<u8>> {
    let mut random = 0x12345678u32;
    (0..12)
        .map(|frame| {
            (0..width * height * 3)
                .map(|index| {
                    let x = index / 3 % width;
                    let y = index / 3 / width;
                    match pattern {
                        0 => ((x + y + frame * 8 + index % 3 * 31) % 256) as u8,
                        1 => {
                            if (x / 32 + y / 32 + frame / 4).is_multiple_of(2) {
                                16
                            } else {
                                235
                            }
                        }
                        _ => {
                            random ^= random << 13;
                            random ^= random >> 17;
                            random ^= random << 5;
                            random as u8
                        }
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
#[ignore]
fn timings() {
    println!("release=true conversion_includes_allocation=true encoder=production_VideoEncoder single_thread=true frames=12 rounds=3");
    for (width, height, bitrate) in [
        (320, 180, 150_000),
        (640, 360, 350_000),
        (960, 540, 600_000),
        (1280, 720, 1_200_000),
        (642, 362, 350_000),
    ] {
        for pattern in 0..3 {
            let frames = frames(width, height, pattern);
            let mut baseline_encoder = baseline_video::VideoEncoder::new(bitrate).unwrap();
            let mut candidate_encoder = video::VideoEncoder::new(bitrate).unwrap();
            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(
                    baseline_encoder
                        .encode(frame, width as u32, height as u32, bitrate, i == 0)
                        .unwrap(),
                    candidate_encoder
                        .encode(frame, width as u32, height as u32, bitrate, i == 0)
                        .unwrap(),
                    "encoded output {width}x{height} pattern={pattern} frame={i}"
                );
            }
            for round in 0..3 {
                for candidate in if round % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let convert = if candidate {
                        video_rgb::convert
                    } else {
                        baseline_rgb::convert
                    };
                    for frame in &frames {
                        let _ = black_box(convert(black_box(frame), width, height));
                    }
                    let start = Instant::now();
                    for _ in 0..8 {
                        for frame in &frames {
                            let _ = black_box(convert(black_box(frame), width, height));
                        }
                    }
                    println!("CONVERT width={width} height={height} pattern={pattern} round={round} candidate={candidate} us_per_frame={:.3}",start.elapsed().as_secs_f64()*1e6/96.0);
                    let start = Instant::now();
                    if candidate {
                        let mut encoder = video::VideoEncoder::new(bitrate).unwrap();
                        for (i, frame) in frames.iter().enumerate() {
                            black_box(
                                encoder
                                    .encode(
                                        black_box(frame),
                                        width as u32,
                                        height as u32,
                                        bitrate,
                                        i == 0,
                                    )
                                    .unwrap(),
                            );
                        }
                    } else {
                        let mut encoder = baseline_video::VideoEncoder::new(bitrate).unwrap();
                        for (i, frame) in frames.iter().enumerate() {
                            black_box(
                                encoder
                                    .encode(
                                        black_box(frame),
                                        width as u32,
                                        height as u32,
                                        bitrate,
                                        i == 0,
                                    )
                                    .unwrap(),
                            );
                        }
                    }
                    println!("ENCODE width={width} height={height} pattern={pattern} round={round} candidate={candidate} us_per_frame={:.3}",start.elapsed().as_secs_f64()*1e6/12.0);
                }
            }
        }
    }
}
