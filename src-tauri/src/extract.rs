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

    let (data, is_color) = if filters.is_empty() {
        // No filters — raw pixel data
        let bpc = img.bits_per_component.unwrap_or(8) as u32;
        let cs = img.color_space.as_deref().unwrap_or("DeviceRGB");
        interpret_raw(content, width, height, cs, bpc)?
    } else if filters.contains(&"DCTDecode".to_string()) {
        // JPEG is self-contained — decode regardless of any other filters
        decode_jpeg(content)?
    } else if filters.contains(&"CCITTFaxDecode".to_string()) {
        let dp = img.origin_dict.get(b"DecodeParms").ok();
        decode_ccitt(content, width, height, dp)?
    } else if filters.contains(&"JPXDecode".to_string()) {
        return Err("JPEG2000 not supported".into());
    } else if filters.contains(&"JBIG2Decode".to_string()) {
        return Err("JBIG2 not supported".into());
    } else {
        // Chain of decompression filters (FlateDecode, RunLengthDecode, ...).
        // Per the PDF spec, filters are applied in array order to decode.
        let mut data = content.to_vec();
        for (i, filter) in filters.iter().enumerate() {
            data = apply_filter(&data, &img.origin_dict, filter, i)?;
        }
        let bpc = img.bits_per_component.unwrap_or(8) as u32;
        let cs = img.color_space.as_deref().unwrap_or("DeviceRGB");
        interpret_raw(&data, width, height, cs, bpc)?
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

/// Apply a single decoding filter to `data`. `filter_index` is the position
/// of this filter within the /Filter array, used to look up the matching
/// entry in /DecodeParms (which is also an array when /Filter is).
fn apply_filter(
    data: &[u8],
    dict: &Dictionary,
    filter: &str,
    filter_index: usize,
) -> Result<Vec<u8>, String> {
    let mut out = match filter {
        "FlateDecode" => {
            use flate2::read::ZlibDecoder;
            use std::io::Read;
            let mut d = ZlibDecoder::new(data);
            let mut out = Vec::new();
            d.read_to_end(&mut out)
                .map_err(|e| format!("FlateDecode: {e}"))?;
            out
        }
        "RunLengthDecode" => decode_runlength(data)?,
        other => return Err(format!("filter {other} not implemented")),
    };

    // Apply a decoding predictor if DecodeParms requests one. The predictor
    // value follows the PNG spec: 10–15 mean one of the PNG row filters
    // (15 = optimal, meaning each row carries its own filter-type byte).
    if let Some((predictor, colors, bpc, columns)) = read_predictor(dict, filter_index) {
        if predictor >= 10 {
            out = depngify(&out, predictor, colors, bpc, columns)?;
        }
    }
    Ok(out)
}

/// RunLengthDecode: a PDF run-length encoding. The data is a sequence of
/// length bytes followed by their content: a length byte n in 0..=127 means
/// "copy the next n+1 bytes literally"; n in 129..=255 means "repeat the
/// next single byte 257-n times". 128 is end-of-data.
fn decode_runlength(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let n = data[i];
        i += 1;
        match n {
            0..=127 => {
                let count = n as usize + 1;
                if i + count > data.len() {
                    return Err("RunLengthDecode: short literal run".into());
                }
                out.extend_from_slice(&data[i..i + count]);
                i += count;
            }
            128 => break, // EOD
            129..=255 => {
                let count = 257 - n as usize;
                if i >= data.len() {
                    return Err("RunLengthDecode: short repeated run".into());
                }
                let b = data[i];
                out.extend(std::iter::repeat_n(b, count));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Extract predictor parameters from a stream's DecodeParms dictionary.
/// `filter_index` selects the matching entry when DecodeParms is an array
/// (one dict per filter in the /Filter array). Returns
/// (predictor, colors, bits_per_component, columns) with PDF defaults applied.
fn read_predictor(dict: &Dictionary, filter_index: usize) -> Option<(u32, u32, u32, u32)> {
    let dp = dict.get(b"DecodeParms").ok()?;
    // DecodeParms may be a single dict or an array of dicts (one per filter).
    let dp_dict = match dp {
        Object::Dictionary(d) => d,
        Object::Array(arr) => {
            let obj = arr.get(filter_index).or_else(|| arr.first())?;
            match obj {
                Object::Dictionary(d) => d,
                _ => return None,
            }
        }
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
    // For sub-byte bpc (1, 2, 4) we expand each sample to one 8-bit byte
    // first, then proceed as if bpc were 8. Samples are stored most-
    // significant-bit-first within each byte.
    let raw = if bpc == 1 || bpc == 2 || bpc == 4 {
        let colors = match color_space {
            "DeviceRGB" => 3,
            "DeviceGray" => 1,
            "DeviceCMYK" => 4,
            _ => return Err(format!("color space {color_space} not supported for raw")),
        };
        expand_samples(raw, width, height, colors, bpc)?
    } else if bpc == 8 {
        raw.to_vec()
    } else {
        return Err(format!("BitsPerComponent {bpc} not supported for raw"));
    };

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
            // Scale the sample to the full 0–255 range so bpc=2/4 images
            // look right (a 2-bit value 3 should be pure white, not 3).
            let scaled = scale_gray(&raw[..expected], bpc);
            Ok((scaled, false))
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

/// Expand sub-byte samples (bpc = 1, 2, or 4) into one byte per sample.
/// Each output byte holds just the sample value (NOT scaled to 0–255).
///
/// Per the PDF spec, image data is organized into rows and **each row
/// begins on a byte boundary** — the trailing bits of the last byte in a
/// row are unused padding and must be skipped. Treating the data as one
/// continuous bit-stream (the naive approach) reads those pad bits as
/// real samples, drifting the decode by one sample per row and producing
/// the classic diagonal-distortion artifact.
fn expand_samples(
    raw: &[u8],
    width: u32,
    height: u32,
    colors: u32,
    bpc: u32,
) -> Result<Vec<u8>, String> {
    let w = width as usize;
    let h = height as usize;
    let samples_per_row = w * colors as usize;
    // Bits used by one row, rounded up to a whole byte = the row stride.
    let bits_per_row = samples_per_row * bpc as usize;
    let row_bytes = bits_per_row.div_ceil(8);
    let total_samples = samples_per_row * h;
    let needed_bytes = row_bytes * h;
    if raw.len() < needed_bytes {
        return Err(format!(
            "expand_samples: need {needed_bytes} bytes for {h} rows of {row_bytes} bytes at {bpc} bpc, got {}",
            raw.len()
        ));
    }

    let mask = (1u8 << bpc) - 1;
    let mut out = Vec::with_capacity(total_samples);

    for row in 0..h {
        let row_start = row * row_bytes;
        // Walk a bit cursor through this row only, decoding samples MSB-first.
        // Because we advance strictly within [0, bits_per_row), the trailing
        // pad bits of the last byte are never read.
        for s in 0..samples_per_row {
            let bit_pos = s * bpc as usize;
            let byte_idx = row_start + (bit_pos >> 3);
            let bit_off = bit_pos & 7;
            // Pull two bytes (MSB-first) so a sample straddling a byte
            // boundary is handled. The extra byte, if any, is within the
            // row's allocated bytes; rows always end on a byte boundary.
            let hi = raw[byte_idx] as u32;
            let lo = if byte_idx + 1 < raw.len() {
                raw[byte_idx + 1] as u32
            } else {
                0
            };
            let window = (hi << 8) | lo;
            let shift = 16 - bit_off - bpc as usize;
            out.push(((window >> shift) as u8) & mask);
        }
    }

    Ok(out)
}

/// Scale gray sample values to the full 0–255 range based on bpc.
/// E.g. for bpc=2 the values 0..=3 map to 0, 85, 170, 255. bpc=8 is a no-op.
fn scale_gray(data: &[u8], bpc: u32) -> Vec<u8> {
    if bpc == 8 {
        return data.to_vec();
    }
    let max_val = (1u32 << bpc) - 1; // 1, 3, 15 for bpc 1, 2, 4
    data.iter()
        .map(|&v| ((v as u32 * 255 + max_val / 2) / max_val).min(255) as u8)
        .collect()
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
