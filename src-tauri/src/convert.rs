use crate::types::*;
use fax::encoder::Encoder;
use fax::{Color, VecWriter};

pub fn convert_to_bw(img: &DecodedImage) -> Result<CompressedImage, String> {
    convert_to_bw_threshold(img, 128)
}

/// Simple threshold — matches ImageMagick `-threshold 50%`.
/// Produces clean bilevel images that compress extremely well with G4.
/// Best choice for scanned documents.
pub fn convert_to_bw_threshold(img: &DecodedImage, threshold: u8) -> Result<CompressedImage, String> {
    let width = img.width;
    let height = img.height;
    let w = width as usize;
    let h = height as usize;

    let bw_pixels: Vec<Color> = if img.is_color {
        (0..w * h)
            .map(|i| {
                let r = img.data[i * 3] as u32;
                let g = img.data[i * 3 + 1] as u32;
                let b = img.data[i * 3 + 2] as u32;
                let lum = (r * 299 + g * 587 + b * 114) / 1000;
                if lum > threshold as u32 {
                    Color::White
                } else {
                    Color::Black
                }
            })
            .collect()
    } else {
        img.data
            .iter()
            .map(|&v| {
                if v > threshold {
                    Color::White
                } else {
                    Color::Black
                }
            })
            .collect()
    };

    encode_g4(&bw_pixels, width, height, img.dpi)
}

/// Floyd-Steinberg error-diffusion dithering.
/// Better visual quality but produces larger G4 output.
pub fn convert_to_bw_dither(img: &DecodedImage) -> Result<CompressedImage, String> {
    let width = img.width;
    let height = img.height;
    let w = width as usize;
    let h = height as usize;

    let mut errors: Vec<f32> = if img.is_color {
        (0..w * h)
            .map(|i| {
                let r = img.data[i * 3] as f32;
                let g = img.data[i * 3 + 1] as f32;
                let b = img.data[i * 3 + 2] as f32;
                0.299 * r + 0.587 * g + 0.114 * b
            })
            .collect()
    } else {
        img.data.iter().map(|&v| v as f32).collect()
    };

    let mut bw_pixels: Vec<Color> = Vec::with_capacity(w * h);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let old = errors[idx];
            let new_val = if old > 128.0 { 255.0 } else { 0.0 };
            bw_pixels.push(if new_val > 128.0 {
                Color::White
            } else {
                Color::Black
            });

            let err = old - new_val;
            if x + 1 < w {
                errors[idx + 1] += err * 7.0 / 16.0;
            }
            if y + 1 < h {
                if x > 0 {
                    errors[idx + w - 1] += err * 3.0 / 16.0;
                }
                errors[idx + w] += err * 5.0 / 16.0;
                if x + 1 < w {
                    errors[idx + w + 1] += err * 1.0 / 16.0;
                }
            }
        }
    }

    encode_g4(&bw_pixels, width, height, img.dpi)
}

fn encode_g4(
    bw_pixels: &[Color],
    width: u32,
    height: u32,
    dpi: f64,
) -> Result<CompressedImage, String> {
    let w = width as usize;
    let h = height as usize;

    let writer = VecWriter::with_capacity(w * h / 8);
    let mut encoder = Encoder::new(writer);

    for y in 0..h {
        let row_start = y * w;
        let row = &bw_pixels[row_start..row_start + w];
        encoder
            .encode_line(row.iter().copied(), width as u16)
            .map_err(|_| "G4 encode error".to_string())?;
    }

    let writer = encoder.finish().map_err(|_| "G4 finish error")?;
    let g4_data = writer.finish();

    Ok(CompressedImage {
        data: g4_data,
        width,
        height,
        color_space: "DeviceGray",
        bits_per_component: 1,
        filter: "CCITTFaxDecode",
        is_ccitt: true,
        dpi,
    })
}

pub fn convert_to_color(img: &DecodedImage, quality: u8) -> Result<CompressedImage, String> {
    let mut jpeg_buf = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, quality);

    let (color_space, color_type) = if img.is_color {
        ("DeviceRGB", image::ExtendedColorType::Rgb8)
    } else {
        ("DeviceGray", image::ExtendedColorType::L8)
    };

    encoder
        .encode(&img.data, img.width, img.height, color_type)
        .map_err(|e| format!("JPEG encode: {e}"))?;

    Ok(CompressedImage {
        data: jpeg_buf,
        width: img.width,
        height: img.height,
        color_space,
        bits_per_component: 8,
        filter: "DCTDecode",
        is_ccitt: false,
        dpi: img.dpi,
    })
}
