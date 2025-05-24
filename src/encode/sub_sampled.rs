use crate::{as_rgba_f32, cast, ch, n1, n8, util, yuv16, yuv8, EncodingError, Report};

use super::encoder::{Args, Encoder, EncoderSet, Flags};

// helpers

fn process_subsample<const BLOCK_WIDTH: usize, EncodedBlock>(
    data: &[[f32; 4]],
    out: &mut [EncodedBlock],
    f: impl Fn(&[[f32; 4]; BLOCK_WIDTH]) -> EncodedBlock,
) {
    let full_blocks_len = data.len() / BLOCK_WIDTH * BLOCK_WIDTH;
    let rest = data.len() - full_blocks_len;

    // process full blocks
    let full = cast::as_array_chunks(&data[..full_blocks_len]).unwrap();
    for (i, o) in full.iter().zip(out.iter_mut()) {
        *o = f(i);
    }

    // process the last partial block (if any)
    if rest > 0 {
        let mut last_block = [[0_f32; 4]; BLOCK_WIDTH];
        last_block[..rest].copy_from_slice(&data[full_blocks_len..]);
        last_block[rest..].fill(data[data.len() - 1]);

        out[full.len()] = f(&last_block);
    }
}
fn uncompressed_universal_subsample<EncodedBlock>(
    args: Args,
    block_width: usize,
    process: fn(usize, &[[f32; 4]], &mut [EncodedBlock]),
) -> Result<(), EncodingError>
where
    EncodedBlock: Default + Copy + cast::ToLe + cast::Castable,
{
    let Args {
        data,
        color,
        writer,
        width,
        height,
        mut progress,
        ..
    } = args;
    let bytes_per_pixel = color.bytes_per_pixel() as usize;

    assert!(block_width >= 2);

    const BUFFER_PIXELS: usize = 512;
    let mut intermediate_buffer = [[0_f32; 4]; BUFFER_PIXELS];
    let mut encoded_buffer = [EncodedBlock::default(); BUFFER_PIXELS / 2];

    let chunk_pixels = BUFFER_PIXELS / block_width * block_width;
    let chunk_size = chunk_pixels * bytes_per_pixel;

    let chunk_count = height * util::div_ceil(width * bytes_per_pixel, chunk_size);
    let mut chunk_index: usize = 0;

    for (y_index, y_line) in data.chunks(width * bytes_per_pixel).enumerate() {
        debug_assert!(y_line.len() == width * bytes_per_pixel);

        for chunk in y_line.chunks(chunk_size) {
            if chunk_index % 4096 == 0 {
                // occasionally report progress
                progress.report(chunk_index as f32 / chunk_count as f32);
            }
            chunk_index += 1;

            let pixels = chunk.len() / bytes_per_pixel;

            let intermediate = &mut intermediate_buffer[..pixels];
            let encoded = &mut encoded_buffer[..util::div_ceil(pixels, block_width)];

            process(y_index, as_rgba_f32(color, chunk, intermediate), encoded);

            cast::ToLe::to_le(encoded);

            writer.write_all(cast::as_bytes(encoded))?;
        }
    }

    Ok(())
}

macro_rules! universal_subsample {
    ($block_width:literal, $out:ty, $f:expr) => {{
        fn process_blocks(_block_y: usize, block: &[[f32; 4]], out: &mut [$out]) {
            process_subsample::<$block_width, $out>(block, out, $f);
        }
        Encoder::new_universal(|args| {
            uncompressed_universal_subsample(args, $block_width, process_blocks)
        })
    }};
}
macro_rules! universal_subsample_dither {
    ($block_width:literal, $out:ty, $f:expr) => {{
        type Block = [[f32; 4]; $block_width];
        fn process_blocks(block_y: usize, block: &[[f32; 4]], out: &mut [$out]) {
            let f = move |block: &Block| -> $out {
                let g = util::closure_types2::<usize, &Block, $out, _>($f);
                g(block_y, block)
            };
            process_subsample::<$block_width, $out>(block, out, f);
        }
        Encoder::new_universal(|args| {
            uncompressed_universal_subsample(args, $block_width, process_blocks)
        })
    }};
}

// encoders

fn to_rgbg([p0, p1]: &[[f32; 4]; 2]) -> [u8; 4] {
    let g0 = n8::from_f32(p0[1]);
    let g1 = n8::from_f32(p1[1]);
    let r = n8::from_f32((p0[0] + p1[0]) * 0.5);
    let b = n8::from_f32((p0[2] + p1[2]) * 0.5);
    [r, g0, b, g1]
}

pub(crate) const R8G8_B8G8_UNORM: EncoderSet =
    EncoderSet::new(&[universal_subsample!(2, [u8; 4], to_rgbg).add_flags(Flags::EXACT_U8)]);

pub(crate) const G8R8_G8B8_UNORM: EncoderSet =
    EncoderSet::new(&[universal_subsample!(2, [u8; 4], |pair| {
        let [r, g0, b, g1] = to_rgbg(pair);
        [g0, r, g1, b]
    })
    .add_flags(Flags::EXACT_U8)]);

fn to_yuy2([p0, p1]: &[[f32; 4]; 2]) -> [u8; 4] {
    let yuv1 = yuv8::from_rgb_f32([p0[0], p0[1], p0[2]]);
    let yuv2 = yuv8::from_rgb_f32([p1[0], p1[1], p1[2]]);
    let y0 = yuv1[0];
    let y1 = yuv2[0];
    fn pick_mid(a: u8, b: u8) -> u8 {
        let a = a as u16;
        let b = b as u16;
        ((a + b) / 2) as u8
    }
    let u = pick_mid(yuv1[1], yuv2[1]);
    let v = pick_mid(yuv1[2], yuv2[2]);
    [y0, u, y1, v]
}

pub(crate) const YUY2: EncoderSet = EncoderSet::new(&[universal_subsample!(2, [u8; 4], to_yuy2)]);

pub(crate) const UYVY: EncoderSet = EncoderSet::new(&[universal_subsample!(2, [u8; 4], |pair| {
    let [y0, u, y1, v] = to_yuy2(pair);
    [u, y0, v, y1]
})]);

fn to_y216([p0, p1]: &[[f32; 4]; 2]) -> [u16; 4] {
    let yuv1 = yuv16::from_rgb_f32([p0[0], p0[1], p0[2]]);
    let yuv2 = yuv16::from_rgb_f32([p1[0], p1[1], p1[2]]);
    let y0 = yuv1[0];
    let y1 = yuv2[0];
    fn pick_mid(a: u16, b: u16) -> u16 {
        let a = a as u32;
        let b = b as u32;
        ((a + b) / 2) as u16
    }
    let u = pick_mid(yuv1[1], yuv2[1]);
    let v = pick_mid(yuv1[2], yuv2[2]);
    [y0, u, y1, v]
}

pub(crate) const Y210: EncoderSet =
    EncoderSet::new(&[
        universal_subsample!(2, [u16; 4], |pair| to_y216(pair).map(|c| c & 0xFFC0))
            .add_flags(Flags::EXACT_U8),
    ]);

pub(crate) const Y216: EncoderSet =
    EncoderSet::new(&[universal_subsample!(2, [u16; 4], to_y216).add_flags(Flags::EXACT_U8)]);

pub(crate) const R1_UNORM: EncoderSet = EncoderSet::new(&[
    universal_subsample!(8, u8, |block| {
        let mut out = 0_u8;
        for (i, &p) in block.iter().enumerate() {
            out |= n1::from_f32(ch::rgba_to_grayscale(p)[0]) << (7 - i);
        }
        out
    }),
    universal_subsample_dither!(8, u8, |block_y, block| {
        #[allow(clippy::eq_op)]
        #[rustfmt::skip]
        const BAYER_8X8: [[f32; 8]; 8] = [
            [0. / 64. - 0.5, 32. / 64. - 0.5, 8. / 64. - 0.5, 40. / 64. - 0.5, 2. / 64. - 0.5, 34. / 64. - 0.5, 10. / 64. - 0.5, 42. / 64. - 0.5],
            [48. / 64. - 0.5, 16. / 64. - 0.5, 56. / 64. - 0.5, 24. / 64. - 0.5, 50. / 64. - 0.5, 18. / 64. - 0.5, 58. / 64. - 0.5, 26. / 64. - 0.5],
            [12. / 64. - 0.5, 44. / 64. - 0.5, 4. / 64. - 0.5, 36. / 64. - 0.5, 14. / 64. - 0.5, 46. / 64. - 0.5, 6. / 64. - 0.5, 38. / 64. - 0.5],
            [60. / 64. - 0.5, 28. / 64. - 0.5, 52. / 64. - 0.5, 20. / 64. - 0.5, 62. / 64. - 0.5, 30. / 64. - 0.5, 54. / 64. - 0.5, 22. / 64. - 0.5],
            [3. / 64. - 0.5, 35. / 64. - 0.5, 11. / 64. - 0.5, 43. / 64. - 0.5, 1. / 64. - 0.5, 33. / 64. - 0.5, 9. / 64. - 0.5, 41. / 64. - 0.5],
            [51. / 64. - 0.5, 19. / 64. - 0.5, 59. / 64. - 0.5, 27. / 64. - 0.5, 49. / 64. - 0.5, 17. / 64. - 0.5, 57. / 64. - 0.5, 25. / 64. - 0.5],
            [15. / 64. - 0.5, 47. / 64. - 0.5, 7. / 64. - 0.5, 39. / 64. - 0.5, 13. / 64. - 0.5, 45. / 64. - 0.5, 5. / 64. - 0.5, 37. / 64. - 0.5],
            [63. / 64. - 0.5, 31. / 64. - 0.5, 55. / 64. - 0.5, 23. / 64. - 0.5, 61. / 64. - 0.5, 29. / 64. - 0.5, 53. / 64. - 0.5, 21. / 64. - 0.5]
        ];
        let bayer = &BAYER_8X8[block_y % 8];

        let mut out = 0_u8;
        for (i, &p) in block.iter().enumerate() {
            out |= n1::from_f32(ch::rgba_to_grayscale(p)[0] + bayer[i]) << (7 - i);
        }
        out
    }).add_flags(Flags::DITHER_COLOR),
]);
