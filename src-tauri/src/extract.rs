use base64::Engine;
use lopdf::{Document, ObjectId, Object, Dictionary};
use lopdf::xobject::PdfImage;

use crate::types::*;

pub fn extract_all_images(path: &str) -> Result<Vec<(u32, DecodedImage)>, String> {
    let doc = Document::load(path).map_err(|e| format!("Failed to load PDF: {e}"))?;
    let pages = doc.get_pages();

    let mut result = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for (&page_num, &page_id) in &pages {
        let dpi = get_page_dpi(&doc, page_id);

        let pdf_images = doc
            .get_page_images(page_id)
            .map_err(|e| format!("Failed to get page images: {e}"))?;

        let mut best: Option<DecodedImage> = None;
        for img in &pdf_images {
            if !seen_ids.insert(img.id) {
                continue;
            }
            match decode_pdf_image(&doc, img, dpi) {
                Ok(decoded) => {
                    if best.as_ref().map_or(true, |b| {
                        decoded.width * decoded.height > b.width * b.height
                    }) {
                        best = Some(decoded);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: skipping image on page {page_num}: {e}");
                }
            }
        }

        if let Some(img) = best {
            result.push((page_num, img));
        }
    }

    Ok(result)
}

fn get_page_dpi(doc: &Document, page_id: ObjectId) -> f64 {
    let media_box = doc.get_dictionary(page_id).and_then(|d| d.get(b"MediaBox"));
    if let Ok(Object::Array(bbox)) = media_box {
        if bbox.len() >= 4 {
            let w = bbox[2].as_f32().unwrap_or(612.0) - bbox[0].as_f32().unwrap_or(0.0);
            let h = bbox[3].as_f32().unwrap_or(792.0) - bbox[1].as_f32().unwrap_or(0.0);
            let pdf_images = doc.get_page_images(page_id).unwrap_or_default();
            if let Some(img) = pdf_images.first() {
                let dpi_x = img.width as f32 / (w / 72.0);
                let dpi_y = img.height as f32 / (h / 72.0);
                return ((dpi_x + dpi_y) / 2.0).round() as f64;
            }
        }
    }
    200.0
}

fn decode_pdf_image(_doc: &Document, img: &PdfImage, dpi: f64) -> Result<DecodedImage, String> {
    let width = img.width as u32;
    let height = img.height as u32;
    if width == 0 || height == 0 {
        return Err("zero dimension image".into());
    }

    let filters: Vec<String> = img.filters.clone().unwrap_or_default();
    let content = img.content;

    let primary_filter = filters.last().map(|s| s.as_str()).unwrap_or("");

    let (data, is_color) = match primary_filter {
        "DCTDecode" => decode_jpeg(content)?,
        "FlateDecode" | "LZWDecode" => {
            let raw = decompress_stream(content, &img.origin_dict, primary_filter)?;
            let bpc = img.bits_per_component.unwrap_or(8) as u32;
            let cs = img.color_space.as_deref().unwrap_or("DeviceRGB");
            interpret_raw(&raw, width, height, cs, bpc)?
        }
        "CCITTFaxDecode" => {
            let dp = img.origin_dict.get(b"DecodeParms").ok();
            decode_ccitt(content, width, height, dp)?
        }
        "JPXDecode" => return Err("JPEG2000 not supported".into()),
        "JBIG2Decode" => return Err("JBIG2 not supported".into()),
        "" => {
            let bpc = img.bits_per_component.unwrap_or(8) as u32;
            let cs = img.color_space.as_deref().unwrap_or("DeviceRGB");
            interpret_raw(content, width, height, cs, bpc)?
        }
        other => return Err(format!("unsupported filter: {other}")),
    };

    Ok(DecodedImage {
        width,
        height,
        data,
        is_color,
        dpi,
    })
}

fn decode_jpeg(data: &[u8]) -> Result<(Vec<u8>, bool), String> {
    let img = image::load_from_memory(data).map_err(|e| format!("JPEG decode: {e}"))?;
    let is_color = matches!(
        img.color(),
        image::ColorType::Rgb8
            | image::ColorType::Rgba8
            | image::ColorType::Rgb16
            | image::ColorType::Rgba16
    );
    if is_color {
        Ok((img.to_rgb8().into_raw(), true))
    } else {
        Ok((img.to_luma8().into_raw(), false))
    }
}

fn decompress_stream(
    content: &[u8],
    dict: &Dictionary,
    filter: &str,
) -> Result<Vec<u8>, String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    let mut out = match filter {
        "FlateDecode" => {
            let mut d = ZlibDecoder::new(content);
            let mut out = Vec::new();
            d.read_to_end(&mut out)
                .map_err(|e| format!("FlateDecode: {e}"))?;
            out
        }
        _ => return Err(format!("decompress: {filter} not implemented")),
    };

    // Apply a decoding predictor if DecodeParms requests one. The predictor
    // value follows the PNG spec: 10–15 mean one of the PNG row filters
    // (15 = optimal, meaning each row carries its own filter-type byte).
    if let Some((predictor, colors, bpc, columns)) = read_predictor(dict) {
        if predictor >= 10 {
            out = depngify(&out, predictor, colors, bpc, columns)?;
        }
    }
    Ok(out)
}

/// Extract predictor parameters from a stream's DecodeParms dictionary.
/// Returns (predictor, colors, bits_per_component, columns) with the PDF
/// defaults applied when fields are absent.
fn read_predictor(dict: &Dictionary) -> Option<(u32, u32, u32, u32)> {
    let dp = dict.get(b"DecodeParms").ok()?;
    // DecodeParms may be a single dict or an array of dicts (one per filter).
    let dp_dict = match dp {
        Object::Dictionary(d) => d,
        Object::Array(arr) => arr.first().and_then(|o| match o {
            Object::Dictionary(d) => Some(d),
            _ => None,
        })?,
        _ => return None,
    };
    let predictor = dp_dict.get(b"Predictor").and_then(|v| v.as_i64()).unwrap_or(1) as u32;
    let colors = dp_dict.get(b"Colors").and_then(|v| v.as_i64()).unwrap_or(1) as u32;
    let bpc = dp_dict.get(b"BitsPerComponent").and_then(|v| v.as_i64()).unwrap_or(8) as u32;
    let columns = dp_dict.get(b"Columns").and_then(|v| v.as_i64()).unwrap_or(1) as u32;
    Some((predictor, colors, bpc, columns))
}

/// Reverse PNG prediction on a decoded FlateDecode stream.
///
/// After zlib inflation, each row is prefixed with a 1-byte filter type
/// followed by the filtered pixel data. We reconstruct the true pixels by
/// applying the inverse of each row's filter using the previous (already
/// reconstructed) row as reference.
fn depngify(
    data: &[u8],
    _predictor: u32,
    colors: u32,
    bpc: u32,
    columns: u32,
) -> Result<Vec<u8>, String> {
    if bpc != 8 {
        return Err(format!(
            "PNG predictor with BitsPerComponent {bpc} not supported"
        ));
    }
    let bpp = colors as usize; // bytes per pixel (bpc/8 * colors)
    let row_bytes = columns as usize * bpp;
    // Each encoded row: 1 filter byte + row_bytes of data.
    let stride = row_bytes + 1;
    if data.len() % stride != 0 {
        return Err(format!(
            "PNG predictor: data length {} not divisible by row stride {}",
            data.len(),
            stride
        ));
    }
    let nrows = data.len() / stride;
    let mut out = vec![0u8; nrows * row_bytes];
    let mut prev_row = vec![0u8; row_bytes];

    for r in 0..nrows {
        let row_start = r * stride;
        let filter = data[row_start];
        let enc = &data[row_start + 1..row_start + 1 + row_bytes];
        let cur = &mut out[r * row_bytes..(r + 1) * row_bytes];

        match filter {
            0 => {
                // None
                cur.copy_from_slice(enc);
            }
            1 => {
                // Sub
                for i in 0..row_bytes {
                    let left = if i >= bpp { cur[i - bpp] } else { 0 };
                    cur[i] = enc[i].wrapping_add(left);
                }
            }
            2 => {
                // Up
                for i in 0..row_bytes {
                    cur[i] = enc[i].wrapping_add(prev_row[i]);
                }
            }
            3 => {
                // Average
                for i in 0..row_bytes {
                    let left = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                    let up = prev_row[i] as u16;
                    cur[i] = enc[i].wrapping_add(((left + up) / 2) as u8);
                }
            }
            4 => {
                // Paeth
                for i in 0..row_bytes {
                    let left = if i >= bpp { cur[i - bpp] } else { 0 };
                    let up = prev_row[i];
                    let upleft = if i >= bpp { prev_row[i - bpp] } else { 0 };
                    cur[i] = enc[i].wrapping_add(paeth(left, up, upleft));
                }
            }
            other => return Err(format!("PNG predictor: unknown row filter {other}")),
        }
        prev_row.copy_from_slice(cur);
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn interpret_raw(
    raw: &[u8],
    width: u32,
    height: u32,
    color_space: &str,
    bpc: u32,
) -> Result<(Vec<u8>, bool), String> {
    if bpc != 8 {
        return Err(format!("BitsPerComponent {bpc} not supported for raw"));
    }

    match color_space {
        "DeviceRGB" => {
            let expected = (width * height * 3) as usize;
            if raw.len() < expected {
                return Err(format!(
                    "raw RGB: expected {expected} bytes, got {}",
                    raw.len()
                ));
            }
            Ok((raw[..expected].to_vec(), true))
        }
        "DeviceGray" => {
            let expected = (width * height) as usize;
            if raw.len() < expected {
                return Err(format!(
                    "raw Gray: expected {expected} bytes, got {}",
                    raw.len()
                ));
            }
            Ok((raw[..expected].to_vec(), false))
        }
        "DeviceCMYK" => {
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for chunk in raw.chunks_exact(4) {
                let c = chunk[0] as f32 / 255.0;
                let m = chunk[1] as f32 / 255.0;
                let y = chunk[2] as f32 / 255.0;
                let k = chunk[3] as f32 / 255.0;
                let r = (1.0 - c) * (1.0 - k) * 255.0;
                let g = (1.0 - m) * (1.0 - k) * 255.0;
                let b = (1.0 - y) * (1.0 - k) * 255.0;
                rgb.extend_from_slice(&[r as u8, g as u8, b as u8]);
            }
            Ok((rgb, true))
        }
        _ => Err(format!("color space {color_space} not supported for raw")),
    }
}

fn decode_ccitt(
    content: &[u8],
    width: u32,
    height: u32,
    decode_parms: Option<&Object>,
) -> Result<(Vec<u8>, bool), String> {
    use lopdf::Dictionary as LoDict;

    // DecodeParms can be a dict, an array of dicts (matching filter array),
    // a reference, or absent (use defaults)
    let dp_dict: &LoDict = match decode_parms {
        Some(Object::Dictionary(d)) => d,
        Some(Object::Array(arr)) => arr
            .first()
            .and_then(|o| match o {
                Object::Dictionary(d) => Some(d),
                _ => None,
            })
            .ok_or("invalid DecodeParms array")?,
        None => {
            // No DecodeParms — use sensible defaults and try Group 4 first
            let mut pixels: Vec<u8> = Vec::with_capacity((width * height) as usize);
            fax::decoder::decode_g4(
                content.iter().copied(),
                width as u16,
                Some(height as u16),
                |transitions| {
                    for pel in fax::decoder::pels(transitions, width as u16) {
                        pixels.push(if pel == fax::Color::Black { 0 } else { 255 });
                    }
                },
            );
            if pixels.len() == (width * height) as usize {
                return Ok((pixels, false));
            }
            // Fallback: Group 3
            pixels.clear();
            fax::decoder::decode_g3(content.iter().copied(), |transitions| {
                for pel in fax::decoder::pels(transitions, width as u16) {
                    pixels.push(if pel == fax::Color::Black { 0 } else { 255 });
                }
            });
            let expected = (width * height) as usize;
            while pixels.len() < expected {
                pixels.push(255);
            }
            return Ok((pixels, false));
        }
        _ => return Err("CCITTFaxDecode: unresolved DecodeParms".into()),
    };

    let k = dp_dict
        .get(b"K")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let columns = dp_dict
        .get(b"Columns")
        .and_then(|v| v.as_i64())
        .unwrap_or(width as i64) as u16;

    let mut pixels: Vec<u8> = Vec::with_capacity((width * height) as usize);

    if k < 0 {
        // Group 4
        fax::decoder::decode_g4(
            content.iter().copied(),
            columns,
            Some(height as u16),
            |transitions| {
                for pel in fax::decoder::pels(transitions, columns) {
                    pixels.push(if pel == fax::Color::Black { 0 } else { 255 });
                }
            },
        );
    } else {
        // Group 3
        fax::decoder::decode_g3(content.iter().copied(), |transitions| {
            for pel in fax::decoder::pels(transitions, columns) {
                pixels.push(if pel == fax::Color::Black { 0 } else { 255 });
            }
        });
    }

    // Pad if decoder produced fewer pixels than expected
    let expected = (width * height) as usize;
    while pixels.len() < expected {
        pixels.push(255); // white
    }

    Ok((pixels, false))
}

pub fn make_thumbnail(img: &DecodedImage) -> String {
    let max_w = 300u32;
    let max_h = 400u32;
    let scale = (max_w as f64 / img.width as f64).min(max_h as f64 / img.height as f64).min(1.0);
    let tw = (img.width as f64 * scale).max(1.0) as u32;
    let th = (img.height as f64 * scale).max(1.0) as u32;

    let thumb = if img.is_color {
        let buf = image::RgbImage::from_raw(img.width, img.height, img.data.clone())
            .ok_or("failed to create image buffer");
        match buf {
            Ok(b) => image::imageops::resize(&b, tw, th, image::imageops::FilterType::Triangle),
            Err(_) => return String::new(),
        }
    } else {
        let buf = image::GrayImage::from_raw(img.width, img.height, img.data.clone())
            .ok_or("failed to create gray buffer");
        match buf {
            Ok(b) => {
                let resized = image::imageops::resize(&b, tw, th, image::imageops::FilterType::Triangle);
                image::DynamicImage::ImageLuma8(resized).to_rgb8()
            }
            Err(_) => return String::new(),
        }
    };

    let mut jpeg_buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 50);
    let _ = enc.encode(
        &thumb,
        tw,
        th,
        image::ExtendedColorType::Rgb8,
    );

    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&jpeg_buf)
    )
}
