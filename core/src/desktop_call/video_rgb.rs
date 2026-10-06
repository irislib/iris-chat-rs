//! Contiguous RGB24 to I420 conversion for desktop call frames.
use openh264::formats::{RgbSliceU8, YUVBuffer};

pub(super) fn convert(rgb: &[u8], width: usize, height: usize) -> YUVBuffer {
    #[cfg(target_arch = "aarch64")]
    if std::arch::is_aarch64_feature_detected!("neon") {
        assert_eq!(width % 2, 0);
        assert_eq!(height % 2, 0);
        let pixels = width.checked_mul(height).expect("RGB dimensions overflow");
        assert_eq!(rgb.len(), pixels.checked_mul(3).expect("RGB size overflow"));
        // SAFETY: NEON is available and both tightly packed source rows and all
        // destination planes are checked before the kernel accesses pointers.
        return unsafe { neon::convert(rgb, width, height, pixels) };
    }
    YUVBuffer::from_rgb8_source(RgbSliceU8::new(rgb, (width, height)))
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::YUVBuffer;
    use std::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    unsafe fn luma(rgb: uint8x16x3_t) -> uint8x16_t {
        let lo = vmlal_u8(
            vmlal_u8(
                vmull_u8(vget_low_u8(rgb.0), vdup_n_u8(66)),
                vget_low_u8(rgb.1),
                vdup_n_u8(129),
            ),
            vget_low_u8(rgb.2),
            vdup_n_u8(25),
        );
        let hi = vmlal_u8(
            vmlal_u8(
                vmull_u8(vget_high_u8(rgb.0), vdup_n_u8(66)),
                vget_high_u8(rgb.1),
                vdup_n_u8(129),
            ),
            vget_high_u8(rgb.2),
            vdup_n_u8(25),
        );
        vaddq_u8(
            vcombine_u8(vshrn_n_u16::<8>(lo), vshrn_n_u16::<8>(hi)),
            vdupq_n_u8(16),
        )
    }

    #[target_feature(enable = "neon")]
    unsafe fn average(top: uint8x16_t, bottom: uint8x16_t) -> int16x8_t {
        let sum = vaddq_u16(vpaddlq_u8(top), vpaddlq_u8(bottom));
        vreinterpretq_s16_u16(vshrq_n_u16::<2>(vaddq_u16(sum, vdupq_n_u16(2))))
    }

    #[target_feature(enable = "neon")]
    unsafe fn chroma(value: int16x8_t) -> uint8x8_t {
        vmovn_u16(vreinterpretq_u16_s16(vaddq_s16(
            vshrq_n_s16::<8>(value),
            vdupq_n_s16(128),
        )))
    }

    /// Match OpenH264 0.9.8's integer coefficients and rounded 2x2 chroma
    /// average exactly. Signed chroma intermediates fit in i16 (±28,560).
    #[target_feature(enable = "neon")]
    pub(super) unsafe fn convert(
        rgb: &[u8],
        width: usize,
        height: usize,
        pixels: usize,
    ) -> YUVBuffer {
        let mut data = vec![0; pixels + pixels / 2];
        let (y, uv) = data.split_at_mut(pixels);
        let (u, v) = uv.split_at_mut(pixels / 4);
        for row in (0..height).step_by(2) {
            let top = &rgb[row * width * 3..][..width * 3];
            let bottom = &rgb[(row + 1) * width * 3..][..width * 3];
            let mut x = 0;
            while x + 16 <= width {
                // Each load consumes exactly 16 RGB24 pixels in its row. Y
                // stores cover those 16 pixels; U/V each cover their 8 pairs.
                let a = vld3q_u8(top.as_ptr().add(x * 3));
                let b = vld3q_u8(bottom.as_ptr().add(x * 3));
                vst1q_u8(y.as_mut_ptr().add(row * width + x), luma(a));
                vst1q_u8(y.as_mut_ptr().add((row + 1) * width + x), luma(b));
                let red = average(a.0, b.0);
                let green = average(a.1, b.1);
                let blue = average(a.2, b.2);
                let uv_index = row / 2 * (width / 2) + x / 2;
                let cb = vmlsq_n_s16(vmlsq_n_s16(vmulq_n_s16(blue, 112), red, 38), green, 74);
                let cr = vmlsq_n_s16(vmlsq_n_s16(vmulq_n_s16(red, 112), blue, 18), green, 94);
                vst1_u8(u.as_mut_ptr().add(uv_index), chroma(cb));
                vst1_u8(v.as_mut_ptr().add(uv_index), chroma(cr));
                x += 16;
            }
            // Even widths need at most seven remaining 2x2 blocks.
            for x in (x..width).step_by(2) {
                let mut sums = [0i16; 3];
                for (dy, source) in [top, bottom].into_iter().enumerate() {
                    for dx in 0..2 {
                        let offset = (x + dx) * 3;
                        let r = i16::from(source[offset]);
                        let g = i16::from(source[offset + 1]);
                        let b = i16::from(source[offset + 2]);
                        y[(row + dy) * width + x + dx] = (((66 * u32::from(source[offset])
                            + 129 * u32::from(source[offset + 1])
                            + 25 * u32::from(source[offset + 2]))
                            >> 8)
                            + 16) as u8;
                        sums[0] += r;
                        sums[1] += g;
                        sums[2] += b;
                    }
                }
                let [r, g, b] = sums.map(|sum| (sum + 2) / 4);
                let index = row / 2 * (width / 2) + x / 2;
                u[index] = (((112 * b - 38 * r - 74 * g) >> 8) + 128) as u8;
                v[index] = (((112 * r - 18 * b - 94 * g) >> 8) + 128) as u8;
            }
        }
        YUVBuffer::from_vec(data, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::formats::YUVSource;

    fn check(rgb: &[u8], width: usize, height: usize) {
        let expected = YUVBuffer::from_rgb8_source(RgbSliceU8::new(rgb, (width, height)));
        let actual = convert(rgb, width, height);
        assert_eq!(actual.y(), expected.y(), "Y at {width}x{height}");
        assert_eq!(actual.u(), expected.u(), "U at {width}x{height}");
        assert_eq!(actual.v(), expected.v(), "V at {width}x{height}");
    }

    #[test]
    fn conversion_matches_openh264_for_tails_colours_and_patterns() {
        let mut random = 123456789u32;
        for width in (2..=66).step_by(2).chain([320, 640, 960, 1280]) {
            for height in [2, 4, 18] {
                let mut rgb = vec![0u8; width * height * 3];
                for colour in [
                    [0, 0, 0],
                    [255, 255, 255],
                    [255, 0, 0],
                    [0, 255, 0],
                    [0, 0, 255],
                ] {
                    for pixel in rgb.as_chunks_mut::<3>().0 {
                        pixel.copy_from_slice(&colour);
                    }
                    check(&rgb, width, height);
                }
                for pattern in 0..3 {
                    for (index, byte) in rgb.iter_mut().enumerate() {
                        random ^= random << 13;
                        random ^= random >> 17;
                        random ^= random << 5;
                        *byte = match pattern {
                            0 => random as u8,
                            1 => {
                                if (index / 3 + index / (width * 3)) % 2 == 0 {
                                    255
                                } else {
                                    0
                                }
                            }
                            _ => index as u8,
                        };
                    }
                    check(&rgb, width, height);
                }
            }
        }
    }

    #[test]
    fn conversion_accepts_empty_frames_and_unaligned_input() {
        for (width, height) in [(0, 0), (0, 4), (4, 0)] {
            check(&[], width, height);
        }
        let storage: Vec<u8> = (0..34 * 6 * 3 + 16).map(|index| index as u8).collect();
        for offset in 0..16 {
            check(&storage[offset..offset + 34 * 6 * 3], 34, 6);
        }
    }

    #[test]
    fn conversion_rejects_odd_dimensions_and_wrong_input_length() {
        for (width, height, len) in [(3, 2, 18), (2, 3, 18), (16, 2, 95), (16, 2, 97)] {
            assert!(std::panic::catch_unwind(|| convert(&vec![0; len], width, height)).is_err());
        }
    }
}
