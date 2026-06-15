use pdf_compress_lib::{convert, extract, pdf_writer, types::DecodedImage};
use lopdf::Document;
use std::path::Path;

fn main() {
    let input = "../scan.pdf";
    let out_dir = Path::new("/tmp/pdf_compare");
    let comp_dir = out_dir.join("compressed");
    std::fs::create_dir_all(&comp_dir).unwrap();

    let pages = extract::extract_all_images(input).expect("extraction failed");

    println!("=== Comparing B&W methods (threshold vs dither) ===\n");
    println!("Page-by-page G4 size comparison:\n");

    let mut threshold_imgs: Vec<_> = Vec::new();
    let mut dither_imgs: Vec<_> = Vec::new();
    let mut smart_imgs: Vec<_> = Vec::new();

    println!("{:<6} {:>10} {:>10} {:>10} {:>10} {:>10}", "Page", "Orig", "Threshold", "Dither", "Color30", "Best");

    for (i, (_, img)) in pages.iter().enumerate() {
        let orig_size = estimate_orig_jpeg_size(&pages, i);

        let thresh = convert::convert_to_bw_threshold(img, 128, 1500).unwrap();
        let dither = convert::convert_to_bw_dither(img, 1500).unwrap();
        let color = convert::convert_to_color(img, 30).unwrap();

        let ts = thresh.data.len();
        let ds = dither.data.len();
        let cs = color.data.len();

        // Smart pick: smallest of threshold and color
        let (best, best_mode, best_size) = if ts <= cs {
            (thresh.clone(), "G4", ts)
        } else {
            (color.clone(), "JPEG", cs)
        };

        println!(
            "{:<6} {:>10} {:>10} {:>10} {:>10} {:>10}",
            format!("p{}", i + 1),
            fmt(orig_size),
            fmt(ts),
            fmt(ds),
            fmt(cs),
            format!("{} {}", best_mode, fmt(best_size)),
        );

        threshold_imgs.push(thresh);
        dither_imgs.push(dither);
        smart_imgs.push(best);
    }

    // Write PDFs
    let thresh_pdf = out_dir.join("threshold.pdf");
    let dither_pdf = out_dir.join("dither.pdf");
    let smart_pdf = out_dir.join("smart_threshold.pdf");

    pdf_writer::write_pdf(&threshold_imgs, thresh_pdf.to_str().unwrap()).unwrap();
    pdf_writer::write_pdf(&dither_imgs, dither_pdf.to_str().unwrap()).unwrap();
    pdf_writer::write_pdf(&smart_imgs, smart_pdf.to_str().unwrap()).unwrap();

    let orig_pdf_size = std::fs::metadata(input).unwrap().len();
    let thresh_size = std::fs::metadata(&thresh_pdf).unwrap().len();
    let dither_size = std::fs::metadata(&dither_pdf).unwrap().len();
    let smart_size = std::fs::metadata(&smart_pdf).unwrap().len();
    let old_size = std::fs::metadata("../old_workflow/compress_pdf/scan.pdf")
        .or_else(|_| std::fs::metadata("old_workflow/compress_pdf/scan.pdf"))
        .map(|m| m.len())
        .unwrap_or(0);

    println!("\n=== FINAL PDF SIZES ===");
    println!("  Original PDF:        {:>8}", fmt_b(orig_pdf_size));
    println!("  Old workflow (IM):   {:>8}  ({:.0}%)", fmt_b(old_size), pct(old_size, orig_pdf_size));
    println!("  Ours threshold:      {:>8}  ({:.0}%)", fmt_b(thresh_size), pct(thresh_size, orig_pdf_size));
    println!("  Ours dither:         {:>8}  ({:.0}%)", fmt_b(dither_size), pct(dither_size, orig_pdf_size));
    println!("  Ours smart(thresh):  {:>8}  ({:.0}%)", fmt_b(smart_size), pct(smart_size, orig_pdf_size));

    // Extract raw streams from best PDF
    println!("\n=== Extracting raw streams from smart threshold PDF ===");
    extract_raw_images(smart_pdf.to_str().unwrap(), &comp_dir);

    println!("\nOpening results...");
    let _ = std::process::Command::new("open")
        .arg(&comp_dir)
        .status();
}

fn estimate_orig_jpeg_size(_pages: &[(u32, DecodedImage)], idx: usize) -> usize {
    // Use the raw extracted sizes from the original PDF
    let doc = Document::load("../scan.pdf").unwrap();
    let page_objs = doc.get_pages();
    let mut count = 0;
    for (&pn, &pid) in &page_objs {
        let imgs = doc.get_page_images(pid).unwrap_or_default();
        if let Some(img) = imgs.iter().max_by_key(|i| i.width * i.height) {
            if count == idx {
                return img.content.len();
            }
            count += 1;
        }
    }
    0
}

fn fmt(n: usize) -> String {
    if n < 1024 {
        format!("{}B", n)
    } else {
        format!("{:.0}K", n as f64 / 1024.0)
    }
}

fn fmt_b(n: u64) -> String {
    if n < 1024 {
        format!("{}B", n)
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}

fn pct(part: u64, whole: u64) -> f64 {
    part as f64 / whole as f64 * 100.0
}

fn extract_raw_images(pdf_path: &str, out_dir: &Path) {
    let doc = Document::load(pdf_path).unwrap();
    let pages = doc.get_pages();

    for (&page_num, &page_id) in &pages {
        let images = doc.get_page_images(page_id).unwrap_or_default();
        if images.is_empty() {
            continue;
        }
        let img = images.iter().max_by_key(|i| i.width * i.height).unwrap();

        let filters: Vec<String> = img.filters.clone().unwrap_or_default();
        let filter = filters.last().map(|s| s.as_str()).unwrap_or("raw");

        match filter {
            "DCTDecode" => {
                let path = out_dir.join(format!("page_{:02}.jpg", page_num));
                std::fs::write(&path, img.content).unwrap();
                println!("  Page {:>2}: JPEG {} -> {}", page_num, fmt(img.content.len()), path.file_name().unwrap().to_str().unwrap());
            }
            "CCITTFaxDecode" => {
                let dp = img.origin_dict.get(b"DecodeParms").ok();
                let tiff = wrap_g4_as_tiff(img.content, img.width as u32, img.height as u32, dp);
                let path = out_dir.join(format!("page_{:02}.tiff", page_num));
                std::fs::write(&path, &tiff).unwrap();
                println!("  Page {:>2}: TIFF/G4 {} -> {}", page_num, fmt(img.content.len()), path.file_name().unwrap().to_str().unwrap());
            }
            _ => {}
        }
    }
}

fn wrap_g4_as_tiff(g4_data: &[u8], width: u32, height: u32, dp: Option<&lopdf::Object>) -> Vec<u8> {
    use lopdf::Object;
    let black_is_1 = dp
        .and_then(|o| match o {
            Object::Dictionary(d) => d.get(b"BlackIs1").ok(),
            Object::Array(a) => a.first().and_then(|o| match o {
                Object::Dictionary(d) => d.get(b"BlackIs1").ok(),
                _ => None,
            }),
            _ => None,
        })
        .and_then(|v| v.as_bool().ok())
        .unwrap_or(false);

    let num_tags = 10u16;
    let ifd_size = 2 + (num_tags as usize * 12) + 4;
    let data_offset = 8 + ifd_size;

    let mut buf = Vec::new();
    buf.extend_from_slice(b"II");
    buf.extend_from_slice(&42u16.to_le_bytes());
    buf.extend_from_slice(&8u32.to_le_bytes());
    buf.extend_from_slice(&num_tags.to_le_bytes());

    let mut tag = |id: u16, typ: u16, count: u32, value: u32| {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&typ.to_le_bytes());
        buf.extend_from_slice(&count.to_le_bytes());
        buf.extend_from_slice(&value.to_le_bytes());
    };

    tag(256, 3, 1, width);
    tag(257, 3, 1, height);
    tag(258, 3, 1, 1);
    tag(259, 3, 1, 4);
    tag(262, 3, 1, if black_is_1 { 1 } else { 0 });
    tag(273, 4, 1, data_offset as u32);
    tag(277, 3, 1, 1);
    tag(278, 3, 1, height);
    tag(279, 4, 1, g4_data.len() as u32);
    tag(293, 4, 1, 0);
    buf.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(buf.len(), data_offset);
    buf.extend_from_slice(g4_data);
    buf
}
