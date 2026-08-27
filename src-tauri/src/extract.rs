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

fn decode_pdf_image(
    doc: &Document,
    img: &PdfImage,
    dpi: f64,
) -> Result<DecodedImage, String> {
    let width = img.width as u32;
    let height = img.height as u32;
    if width == 0 || height == 0 {
        return Err("zero dimension image".into());
    }
    let filters: Vec<String> = img.filters.clone().unwrap_or_default();
    let content = img.content;

    // "Terminal" image formats — self-describing bitstreams (JPEG, JBIG2) or
    // fax (CCITT) — can't go through the raw-pixel interpreter. A compression
    // filter (e.g. FlateDecode) may sit earlier in the /Filter array and must
    // be unwound first: real PDFs wrap JPEG in FlateDecode
    // (/Filter [/FlateDecode /DCTDecode]), and feeding still-zlib bytes to the
    // JPEG decoder silently drops the page.
    let terminal_idx = filters.iter().position(|f| {
        matches!(
            f.as_str(),
            "DCTDecode" | "JPXDecode" | "JBIG2Decode" | "CCITTFaxDecode"
        )
    });

    if let Some(idx) = terminal_idx {
        // Walk the compression chain preceding the terminal format.
        let mut data = content.to_vec();
        for (i, filter) in filters[..idx].iter().enumerate() {
            data = apply_filter(&data, &img.origin_dict, filter, i)?;
        }
        let (data, is_color, w, h) = match filters[idx].as_str() {
            // JPEG — hand the decompressed bytes to the image crate. Uses the
            // JPEG's actual dimensions (can differ from the dict), keeping
            // grayscale JPEGs gray (is_color=false) for better G4 re-encode.
            "DCTDecode" => {
                let (data, is_color, w, h) = decode_jpeg(&data)?;
                (data, is_color, w, h)
            }
            "JPXDecode" => return Err("JPEG2000 not supported".into()),
            // JBIG2 — 1-bit bilevel scans. Optional /JBIG2Globals shared
            // symbol-dictionary bytes precede the page stream (T.88 §7.5).
            // Dimensions come from the codestream, which can differ from the
            // image dict's W/H (e.g. padding rows).
            "JBIG2Decode" => {
                let globals = jbig2_globals(doc, img);
                let (gray, w, h) = decode_jbig2(&data, globals.as_deref())?;
                (gray, false, w, h)
            }
            // CCITT (fax) — B&W scans, grayscale 8-bit output.
            "CCITTFaxDecode" => {
                let dp = img.origin_dict.get(b"DecodeParms").ok();
                let (gray, _) = decode_ccitt(&data, width, height, dp, idx)?;
                (gray, false, width, height)
            }
            _ => unreachable!("terminal_idx only matches the four filters above"),
        };
        return Ok(DecodedImage {
            width: w,
            height: h,
            data,
            is_color,
            dpi,
        });
    }

    // Otherwise a chain of compression filters ending in raw pixel data,
    // interpreted by color space + bpc. Color space is resolved against the
    // document (indirect refs, ICCBased /N, CalGray/CalRGB) because lopdf's
    // PdfImage::color_space is None for indirect refs and only yields the
    // family name for arrays.
    let mut data = content.to_vec();
    for (i, filter) in filters.iter().enumerate() {
        data = apply_filter(&data, &img.origin_dict, filter, i)?;
    }
    let bpc = img.bits_per_component.unwrap_or(8) as u32;
    let cs = resolve_color_space(doc, &img.origin_dict)
        .or_else(|| img.color_space.clone())
        .unwrap_or_else(|| "DeviceRGB".to_string());
    let (data, is_color) = interpret_raw(&data, width, height, &cs, bpc)?;

    Ok(DecodedImage {
        width,
        height,
        data,
        is_color,
        dpi,
    })
}

/// Decode a JPEG stream. Returns (pixels, is_color, width, height) using the
/// JPEG's OWN dimensions from its SOF header — these can disagree with the
/// image dict's /Width /Height (patched JPEGs), and the buffer length must
/// stay consistent with the returned dims or downstream `from_raw` panics.
fn decode_jpeg(data: &[u8]) -> Result<(Vec<u8>, bool, u32, u32), String> {
    let img = image::load_from_memory(data).map_err(|e| format!("JPEG decode: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let is_color = matches!(
        img.color(),
        image::ColorType::Rgb8
            | image::ColorType::Rgba8
            | image::ColorType::Rgb16
            | image::ColorType::Rgba16
    );
    if is_color {
        Ok((img.to_rgb8().into_raw(), true, w, h))
    } else {
        Ok((img.to_luma8().into_raw(), false, w, h))
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
        "LZWDecode" => {
            // PDF defaults to EarlyChange=1 (Adobe PDF Ref §7.4.4.1): the code
            // width increases one code earlier than "original" LZW. In weezl
            // that is `with_tiff_size_switch`; plain `Decoder::new` is
            // EarlyChange=0 and corrupts silently past 9-bit codes.
            use weezl::{decode::Decoder, BitOrder};
            let mut dec = Decoder::with_tiff_size_switch(BitOrder::Msb, 8);
            dec.decode(data).map_err(|e| format!("LZWDecode: {e:?}"))?
        }
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

/// Reverse PNG prediction on a decoded stream. Each row carries a 1-byte
/// filter type (None/Sub/Up/Average/Paeth) followed by the filtered pixels.
///
/// PNG filters operate on bytes, not samples. The "bytes per pixel" used as
/// the Sub/Average/Paeth left-neighbor distance is `ceil(colors * bpc / 8)`;
/// for the common 8-bpc case that's just `colors`. Each row is
/// `ceil(columns * colors * bpc / 8)` bytes wide — sub-byte samples (bpc
/// 1/2/4) pack left-to-right within each row and every row begins on a byte
/// boundary, exactly matching what `expand_samples` expects downstream.
fn depngify(
    data: &[u8],
    _predictor: u32,
    colors: u32,
    bpc: u32,
    columns: u32,
) -> Result<Vec<u8>, String> {
    if !(1..=16).contains(&bpc) {
        return Err(format!("PNG predictor with BitsPerComponent {bpc} not supported"));
    }
    let bits_per_pixel = colors.checked_mul(bpc).ok_or("colors*bpc overflow")? as usize;
    // ceil — a 1-bpc grayscale pixel is 1 bit, bpp rounds up to 1 byte.
    let bpp = bits_per_pixel.div_ceil(8);
    let row_bytes = (columns as usize)
        .checked_mul(bits_per_pixel)
        .ok_or("columns*bits overflow")?
        .div_ceil(8);
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
    // For sub-byte bpc (1, 2, 4) we expand each sample to one byte and
    // stretch to the full 0–255 range — for EVERY color space, not just
    // gray. Skipping the stretch on RGB leaves each channel at 0..2^bpc-1
    // (e.g. 0–3 for 2-bpc scans), rendering as a solid black page.
    let raw = if bpc == 1 || bpc == 2 || bpc == 4 {
        let colors = match color_space {
            "DeviceRGB" => 3,
            "DeviceGray" => 1,
            "DeviceCMYK" => 4,
            _ => return Err(format!("color space {color_space} not supported for raw")),
        };
        let expanded = expand_samples(raw, width, height, colors, bpc)?;
        scale_gray(&expanded, bpc)
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

/// Scale sub-byte sample values (already expanded to one byte each) to the
/// full 0–255 range based on bpc. Color-space agnostic. E.g. for bpc=2 the
/// values 0..=3 map to 0, 85, 170, 255. bpc=8 is a no-op.
fn scale_gray(data: &[u8], bpc: u32) -> Vec<u8> {
    if bpc == 8 {
        return data.to_vec();
    }
    let max_val = (1u32 << bpc) - 1; // 1, 3, 15 for bpc 1, 2, 4
    data.iter()
        .map(|&v| ((v as u32 * 255 + max_val / 2) / max_val).min(255) as u8)
        .collect()
}

/// Decode a CCITT (fax) encoded image to 8-bit grayscale.
///
/// `/BlackIs1` (PDF spec §7.4.6) controls decoded bit polarity: `false` (the
/// default) means a `1` bit is white; `true` means a `1` bit is black. The
/// `fax` crate decodes to semantic Color::Black/White assuming BlackIs1=false
/// semantics, so when the stream declares BlackIs1=true we flip the mapping —
/// otherwise the page comes out white-on-black.
fn decode_ccitt(
    content: &[u8],
    width: u32,
    height: u32,
    decode_parms: Option<&Object>,
    filter_index: usize,
) -> Result<(Vec<u8>, bool), String> {
    // Read a scalar DecodeParms field from either a dict or array-of-dicts.
    // In the array form the entry must be indexed by the CCITT filter's
    // position in /Filter — chains like [/FlateDecode /CCITTFaxDecode] carry
    // [/flate-dict /ccitt-dict], and reading arr.first() would apply the
    // Flate parms (K/Columns/BlackIs1 all defaulted) to a fax stream.
    let parm = |key: &[u8]| -> Option<lopdf::Object> {
        let dp = decode_parms?;
        let d = match dp {
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
        d.get(key).ok().cloned()
    };

    let columns: u16 = parm(b"Columns")
        .and_then(|v| v.as_i64().ok())
        .unwrap_or(width as i64) as u16;
    // /BlackIs1 defaults to false. A bare boolean or an integer (1 = true)
    // both appear in the wild.
    let black_is_1 = match parm(b"BlackIs1") {
        Some(Object::Boolean(b)) => b,
        Some(o) => o.as_i64().map(|v| v != 0).unwrap_or(false),
        None => false,
    };

    let mut pixels: Vec<u8> = Vec::with_capacity((width * height) as usize);
    let decode = |transitions: &[u16], px: &mut Vec<u8>| {
        for pel in fax::decoder::pels(transitions, columns) {
            // fax's Color semantics match BlackIs1=false (1=white). When the
            // stream declares BlackIs1=true, flip black↔white.
            let is_black = pel == fax::Color::Black;
            let ink = if black_is_1 { !is_black } else { is_black };
            px.push(if ink { 0 } else { 255 });
        }
    };

    let k: i64 = parm(b"K").and_then(|v| v.as_i64().ok()).unwrap_or(0);
    if k < 0 {
        // Group 4
        fax::decoder::decode_g4(content.iter().copied(), columns, Some(height as u16), |t| {
            decode(t, &mut pixels);
        });
    } else {
        // Group 3. With no DecodeParms at all, try G4 first then fall back —
        // some producers omit the dict while still writing G4 streams.
        if decode_parms.is_none() {
            fax::decoder::decode_g4(content.iter().copied(), columns, Some(height as u16), |t| {
                decode(t, &mut pixels);
            });
            if pixels.len() != (width * height) as usize {
                pixels.clear();
                fax::decoder::decode_g3(content.iter().copied(), |t| {
                    decode(t, &mut pixels);
                });
            }
        } else {
            fax::decoder::decode_g3(content.iter().copied(), |t| {
                decode(t, &mut pixels);
            });
        }
    }

    // Reconcile the decoded length with width*height. decode_g3 has no row
    // cap (it decodes until RTC/error), so a stream yielding more lines than
    // /Height — or /Columns > /Width — overruns; a truncated stream underruns.
    // Either mismatch would panic the downstream `from_raw(...).expect()` in
    // convert.rs, so pad AND truncate here.
    let expected = (width * height) as usize;
    while pixels.len() < expected {
        pixels.push(255); // white
    }
    pixels.truncate(expected);

    Ok((pixels, false))
}

/// Decode a JBIG2-encoded image to 8-bit grayscale (0 = black, 255 = white).
///
/// `globals` is the optional `/JBIG2Globals` stream bytes. JBIG2 is black=1
/// (opposite of PDF gray convention), so the Decoder impl writes 0x00 for
/// black and leaves the default 0xFF white.
///
/// Returns the pixels plus the image's *actual* dimensions from the
/// codestream — NOT the image dict's W/H, which can disagree (e.g. trailing
/// padding rows); sizing the buffer by the dict would panic.
fn decode_jbig2(
    data: &[u8],
    globals: Option<&[u8]>,
) -> Result<(Vec<u8>, u32, u32), String> {
    let image = hayro_jbig2::Image::new_embedded(data, globals)
        .map_err(|e| format!("JBIG2: {e:?}"))?;
    let (width, height) = (image.width(), image.height());

    // 8-bit gray buffer defaulting to white; the writer only writes black.
    let mut out = vec![0xFFu8; (width as usize) * (height as usize)];

    struct Gray8Writer<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl hayro_jbig2::Decoder for Gray8Writer<'_> {
        fn push_pixel(&mut self, black: bool) {
            if black && self.pos < self.buf.len() {
                self.buf[self.pos] = 0x00;
            }
            self.pos += 1;
        }
        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            let n = (chunk_count as usize) * 8;
            if black {
                let end = (self.pos + n).min(self.buf.len());
                if self.pos < end {
                    self.buf[self.pos..end].fill(0x00);
                }
            }
            self.pos += n;
        }
        fn next_line(&mut self) {}
    }

    let mut writer = Gray8Writer { buf: &mut out, pos: 0 };
    image
        .decode(&mut writer)
        .map_err(|e| format!("JBIG2 decode: {e:?}"))?;

    Ok((out, width, height))
}

/// Resolve the optional `/JBIG2Globals` shared symbol-dictionary stream for a
/// JBIG2 image. The stream may itself be Flate-compressed;
/// `decompressed_content` applies its filter chain, matching what
/// poppler/mupdf feed their decoders. Failures return None — decode proceeds
/// without the dictionary.
fn jbig2_globals(doc: &Document, img: &PdfImage) -> Option<Vec<u8>> {
    let is_jbig2 = img
        .filters
        .as_ref()
        .map(|fs| fs.iter().any(|f| f == "JBIG2Decode"))
        .unwrap_or(false);
    if !is_jbig2 {
        return None;
    }

    // /DecodeParms may be a single dict or an array aligned to /Filter.
    let dp = img.origin_dict.get(b"DecodeParms").ok()?;
    let dp_dict = match dp {
        Object::Dictionary(d) => d,
        Object::Array(arr) => {
            // Prefer the entry at the JBIG2 filter's index; fall back to the
            // first dict carrying a JBIG2Globals key.
            let jbig2_idx = img
                .filters
                .as_ref()
                .and_then(|fs| fs.iter().position(|f| f == "JBIG2Decode"));
            jbig2_idx
                .and_then(|i| match arr.get(i)? {
                    Object::Dictionary(d) => Some(d),
                    _ => None,
                })
                .or_else(|| {
                    arr.iter().filter_map(|o| match o {
                        Object::Dictionary(d) => Some(d),
                        _ => None,
                    }).find(|d| d.get(b"JBIG2Globals").is_ok())
                })?
        }
        _ => return None,
    };

    let globals_ref = match dp_dict.get(b"JBIG2Globals").ok()? {
        Object::Reference(id) => *id,
        _ => return None,
    };

    let stream = doc.get_object(globals_ref).and_then(|o| o.as_stream()).ok()?;
    stream.decompressed_content().ok()
}

/// Map a bare color-space name to itself if it is one of the three device
/// spaces, else None.
fn device_name(name: &str) -> Option<String> {
    match name {
        "DeviceGray" | "DeviceRGB" | "DeviceCMYK" => Some(name.to_string()),
        _ => None,
    }
}

/// Resolve an image XObject's /ColorSpace to the device-space name
/// interpret_raw understands. lopdf's PdfImage::color_space is None for
/// indirect refs (`/ColorSpace 6 0 R`, what macOS Quartz writes) and only
/// yields the family name ("ICCBased") for arrays — neither is usable.
///
/// Resolution follows PDF §8.6: bare device names pass through; ICCBased maps
/// by the ICC stream's /N (1/3/4 → Gray/RGB/CMYK) with /Alternate fallback;
/// CalGray/CalRGB share their device counterparts' layout; everything else
/// (Indexed, Separation, DeviceN, Lab, Pattern) → None → clear error.
fn resolve_color_space(doc: &Document, dict: &Dictionary) -> Option<String> {
    let cs = dict.get(b"ColorSpace").ok()?;
    // Indirect reference — resolve one level.
    let cs = match cs {
        Object::Reference(id) => doc.get_object(*id).ok()?,
        _ => cs,
    };
    match cs {
        Object::Name(n) => device_name(&String::from_utf8_lossy(n)),
        Object::Array(arr) => {
            let family = String::from_utf8_lossy(arr.first()?.as_name().ok()?).into_owned();
            match family.as_str() {
                "CalGray" => Some("DeviceGray".into()),
                "CalRGB" => Some("DeviceRGB".into()),
                "ICCBased" => {
                    // [/ICCBased <stream-ref>]: the ICC stream's /N is the
                    // component count; /Alternate is the fallback.
                    let stream = match arr.get(1)? {
                        Object::Reference(id) => {
                            doc.get_object(*id).ok()?.as_stream().ok()?
                        }
                        _ => return None,
                    };
                    match stream.dict.get(b"N").and_then(|v| v.as_i64()) {
                        Ok(1) => Some("DeviceGray".into()),
                        Ok(3) => Some("DeviceRGB".into()),
                        Ok(4) => Some("DeviceCMYK".into()),
                        _ => {
                            let alt = stream.dict.get(b"Alternate").ok()?;
                            device_name(&String::from_utf8_lossy(alt.as_name().ok()?))
                        }
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
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
