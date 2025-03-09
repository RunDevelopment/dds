// helpers

use crate::{cast, ch, convert_to_rgba_f32, n4, util, ColorFormatSet};

use super::{
    bc1, bc4, bcn_util,
    write::{BaseEncoder, Flags},
    Args, DecodedArgs, EncodeError, EncodeOptions,
};

fn block_universal<
    const BLOCK_WIDTH: usize,
    const BLOCK_HEIGHT: usize,
    const BLOCK_BYTES: usize,
>(
    args: Args,
    encode_block: fn(&[[f32; 4]], usize, &EncodeOptions, &mut [u8; BLOCK_BYTES]),
) -> Result<(), EncodeError> {
    let DecodedArgs {
        data,
        color,
        writer,
        width,
        options,
        ..
    } = DecodedArgs::from(args)?;
    let bytes_per_pixel = color.bytes_per_pixel() as usize;

    let mut intermediate_buffer = vec![[0_f32; 4]; width * BLOCK_HEIGHT];
    let mut encoded_buffer = vec![[0_u8; BLOCK_BYTES]; util::div_ceil(width, BLOCK_WIDTH)];

    let row_pitch = width * bytes_per_pixel;
    for line_group in data.chunks(row_pitch * BLOCK_HEIGHT) {
        debug_assert!(line_group.len() % row_pitch == 0);
        let rows_in_group = line_group.len() / row_pitch;

        // fill the intermediate buffer
        convert_to_rgba_f32(
            color,
            line_group,
            &mut intermediate_buffer[..rows_in_group * width],
        );
        for i in 0..(BLOCK_HEIGHT - rows_in_group) {
            // copy the first line to fill the rest
            intermediate_buffer.copy_within(..width, (rows_in_group + i) * width);
        }

        // handle full blocks
        for block_index in 0..width / BLOCK_WIDTH {
            let block_start = block_index * BLOCK_WIDTH;
            let block = &intermediate_buffer[block_start..];
            let encoded = &mut encoded_buffer[block_index];

            encode_block(block, width, &options, encoded);
        }

        // handle last partial block
        if width % BLOCK_WIDTH != 0 {
            let block_index = width / BLOCK_WIDTH;
            let block_start = block_index * BLOCK_WIDTH;
            let block_width = width - block_start;

            // fill block data
            let mut block_data = vec![[0_f32; 4]; BLOCK_WIDTH * BLOCK_HEIGHT];
            for i in 0..BLOCK_HEIGHT {
                let row = &mut block_data[i * BLOCK_WIDTH..(i + 1) * BLOCK_WIDTH];
                let partial_row = &intermediate_buffer[block_start + i * width..][..block_width];
                row[..block_width].copy_from_slice(partial_row);
                let last = partial_row.last().copied().unwrap_or_default();
                row[block_width..].fill(last);
            }

            let encoded = &mut encoded_buffer[block_index];
            encode_block(&block_data, BLOCK_WIDTH, &options, encoded);
        }

        writer.write_all(cast::as_bytes(&encoded_buffer))?;
    }

    Ok(())
}

fn get_4x4_rgba(data: &[[f32; 4]], row_pitch: usize) -> [[f32; 4]; 16] {
    let mut block: [[f32; 4]; 16] = [[0.0; 4]; 16];
    for i in 0..4 {
        for j in 0..4 {
            block[i * 4 + j] = data[i * row_pitch + j];
        }
    }
    block
}
fn get_4x4_grayscale(data: &[[f32; 4]], row_pitch: usize) -> [f32; 16] {
    let mut block = [0.0; 16];
    for i in 0..4 {
        for j in 0..4 {
            block[i * 4 + j] = ch::rgba_to_grayscale(data[i * row_pitch + j])[0];
        }
    }
    block
}
fn get_4x4_select_channel<const CHANNEL: usize>(data: &[[f32; 4]], row_pitch: usize) -> [f32; 16] {
    let mut block = [0.0; 16];
    for i in 0..4 {
        for j in 0..4 {
            block[i * 4 + j] = data[i * row_pitch + j][CHANNEL];
        }
    }
    block
}

fn get_alpha(data: &[[f32; 4]; 16]) -> [f32; 16] {
    data.map(|x| x[3])
}
fn pre_multiply_alpha(data: &mut [[f32; 4]]) {
    for pixel in data.iter_mut() {
        let [r, g, b, a] = pixel.map(|x| x.clamp(0.0, 1.0));
        pixel[0] = r * a;
        pixel[1] = g * a;
        pixel[2] = b * a;
        pixel[3] = a;
    }
}

fn concat_blocks(left: [u8; 8], right: [u8; 8]) -> [u8; 16] {
    let mut out = [0; 16];
    out[0..8].copy_from_slice(&left);
    out[8..].copy_from_slice(&right);
    out
}

// encoders

fn get_bc1_options(options: &EncodeOptions) -> bc1::Bc1Options {
    let mut o = bc1::Bc1Options::default();
    o.dither = options.dithering.color();
    o
}
pub const BC1_UNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_ALL,
    encode: |args| {
        block_universal::<4, 4, 8>(args, |data, row_pitch, options, out| {
            let bc1_options = get_bc1_options(options);
            let mut block = get_4x4_rgba(data, row_pitch);

            if options.dithering.alpha() {
                let alpha = get_alpha(&block);
                bcn_util::block_dither(&alpha, |i, pixel| {
                    let alpha = if pixel >= 0.5 { 1.0 } else { 0.0 };
                    block[i][3] = alpha;
                    alpha
                });
            }

            *out = bc1::compress_bc1_block(block, bc1_options);
        })
    },
}];

fn bc2_alpha(alpha: [f32; 16], options: &EncodeOptions) -> [u8; 8] {
    let mut indexes: u64 = 0;
    let mut set_value = |i: usize, value: u8| {
        debug_assert!(value < 16);
        indexes |= (value as u64) << (i * 4);
    };

    if options.dithering.alpha() {
        bcn_util::block_dither(&alpha, |i, pixel| {
            let value = n4::from_f32(pixel);
            set_value(i, value);
            n4::f32(value)
        });
    } else {
        for (i, pixel) in alpha.into_iter().enumerate() {
            let value = n4::from_f32(pixel);
            set_value(i, value);
        }
    }

    indexes.to_le_bytes()
}

pub const BC2_UNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_ALL,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let (bc1_options, _) = get_bc3_options(options);

            let block = get_4x4_rgba(data, row_pitch);

            let alpha_block = bc2_alpha(get_alpha(&block), options);
            let bc1_block = bc1::compress_bc1_block(block, bc1_options);

            *out = concat_blocks(alpha_block, bc1_block);
        })
    },
}];
pub const BC2_UNORM_PREMULTIPLIED_ALPHA: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_ALL,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let (bc1_options, _) = get_bc3_options(options);

            let mut block = get_4x4_rgba(data, row_pitch);
            pre_multiply_alpha(&mut block);

            let alpha_block = bc2_alpha(get_alpha(&block), options);
            let bc1_block = bc1::compress_bc1_block(block, bc1_options);

            *out = concat_blocks(alpha_block, bc1_block);
        })
    },
}];

fn get_bc3_options(options: &EncodeOptions) -> (bc1::Bc1Options, bc4::Bc4Options) {
    let mut bc1_options = get_bc1_options(options);
    bc1_options.no_default = true;

    let mut bc4_options = get_bc4_options(options);
    bc4_options.snorm = false;

    (bc1_options, bc4_options)
}
pub const BC3_UNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_ALL,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let (bc1_options, bc4_options) = get_bc3_options(options);

            let block = get_4x4_rgba(data, row_pitch);

            let bc4_block = bc4::compress_bc4_block(get_alpha(&block), bc4_options);
            let bc1_block = bc1::compress_bc1_block(block, bc1_options);

            *out = concat_blocks(bc4_block, bc1_block);
        })
    },
}];
pub const BC3_UNORM_PREMULTIPLIED_ALPHA: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_ALL,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let (bc1_options, bc4_options) = get_bc3_options(options);

            let mut block = get_4x4_rgba(data, row_pitch);
            pre_multiply_alpha(&mut block);

            let bc4_block = bc4::compress_bc4_block(get_alpha(&block), bc4_options);
            let bc1_block = bc1::compress_bc1_block(block, bc1_options);

            *out = concat_blocks(bc4_block, bc1_block);
        })
    },
}];

fn handle_bc4(data: &[[f32; 4]], row_pitch: usize, options: bc4::Bc4Options) -> [u8; 8] {
    let block = get_4x4_grayscale(data, row_pitch);
    bc4::compress_bc4_block(block, options)
}
fn get_bc4_options(options: &EncodeOptions) -> bc4::Bc4Options {
    bc4::Bc4Options {
        dither: options.dithering.color(),
        snorm: false,
    }
}

pub const BC4_UNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_COLOR,
    encode: |args| {
        block_universal::<4, 4, 8>(args, |data, row_pitch, options, out| {
            let mut options = get_bc4_options(options);
            options.snorm = false;
            *out = handle_bc4(data, row_pitch, options);
        })
    },
}];

pub const BC4_SNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_COLOR,
    encode: |args| {
        block_universal::<4, 4, 8>(args, |data, row_pitch, options, out| {
            let mut options = get_bc4_options(options);
            options.snorm = true;
            *out = handle_bc4(data, row_pitch, options);
        })
    },
}];

fn handle_bc5(data: &[[f32; 4]], row_pitch: usize, options: bc4::Bc4Options) -> [u8; 16] {
    let red_block = get_4x4_select_channel::<0>(data, row_pitch);
    let green_block = get_4x4_select_channel::<1>(data, row_pitch);

    let red = bc4::compress_bc4_block(red_block, options);
    let green = bc4::compress_bc4_block(green_block, options);

    concat_blocks(red, green)
}

pub const BC5_UNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_COLOR,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let mut options = get_bc4_options(options);
            options.snorm = false;
            *out = handle_bc5(data, row_pitch, options);
        })
    },
}];

pub const BC5_SNORM: &[BaseEncoder] = &[BaseEncoder {
    color_formats: ColorFormatSet::ALL,
    flags: Flags::DITHER_COLOR,
    encode: |args| {
        block_universal::<4, 4, 16>(args, |data, row_pitch, options, out| {
            let mut options = get_bc4_options(options);
            options.snorm = true;
            *out = handle_bc5(data, row_pitch, options);
        })
    },
}];
