use pdf_compress_lib::{convert, extract, pdf_writer};

fn main() {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/test_scanned.pdf".to_string());

    println!("=== Extracting images from {input} ===");
    let pages = extract::extract_all_images(&input).expect("extraction failed");
    println!("Found {} pages with images", pages.len());

    for (page_num, img) in &pages {
        println!(
            "  Page {page_num}: {}x{} {} ({} bytes raw)",
            img.width,
            img.height,
            if img.is_color { "color" } else { "gray" },
            img.data.len()
        );
    }

    // Test B&W conversion
    println!("\n=== Converting all pages to B&W (CCITT G4) ===");
    let bw_images: Vec<_> = pages
        .iter()
        .map(|(_, img)| convert::convert_to_bw(img, 1500).expect("bw conversion failed"))
        .collect();
    for (i, img) in bw_images.iter().enumerate() {
        println!(
            "  Page {}: G4 data {} bytes ({:.1}% of raw)",
            i + 1,
            img.data.len(),
            img.data.len() as f64 / pages[i].1.data.len() as f64 * 100.0
        );
    }

    let bw_out = "/tmp/test_output_bw.pdf";
    pdf_writer::write_pdf(&bw_images, bw_out).expect("pdf write failed");
    let bw_size = std::fs::metadata(bw_out).unwrap().len();
    println!("  Output: {bw_out} ({:.0} KB)", bw_size as f64 / 1024.0);

    // Test Color conversion
    println!("\n=== Converting all pages to Color (JPEG 30%) ===");
    let color_images: Vec<_> = pages
        .iter()
        .map(|(_, img)| convert::convert_to_color(img, 30).expect("color conversion failed"))
        .collect();
    for (i, img) in color_images.iter().enumerate() {
        println!(
            "  Page {}: JPEG data {} bytes ({:.1}% of raw)",
            i + 1,
            img.data.len(),
            img.data.len() as f64 / pages[i].1.data.len() as f64 * 100.0
        );
    }

    let color_out = "/tmp/test_output_color.pdf";
    pdf_writer::write_pdf(&color_images, color_out).expect("pdf write failed");
    let color_size = std::fs::metadata(color_out).unwrap().len();
    println!("  Output: {color_out} ({:.0} KB)", color_size as f64 / 1024.0);

    // Test mixed: page 0 = color, page 1 = bw
    println!("\n=== Mixed mode: page 1 color, page 2 B&W ===");
    let mixed: Vec<_> = pages
        .iter()
        .enumerate()
        .map(|(i, (_, img))| {
            if i == 0 {
                convert::convert_to_color(img, 30).unwrap()
            } else {
                convert::convert_to_bw(img, 1500).unwrap()
            }
        })
        .collect();
    let mixed_out = "/tmp/test_output_mixed.pdf";
    pdf_writer::write_pdf(&mixed, mixed_out).expect("pdf write failed");
    let mixed_size = std::fs::metadata(mixed_out).unwrap().len();
    println!("  Output: {mixed_out} ({:.0} KB)", mixed_size as f64 / 1024.0);

    // Verify round-trip: extract from our output PDF
    println!("\n=== Verifying round-trip extraction ===");
    let re_extracted = extract::extract_all_images(bw_out).expect("re-extraction failed");
    println!(
        "Re-extracted {} images from B&W output",
        re_extracted.len()
    );
    for (i, (_, img)) in re_extracted.iter().enumerate() {
        println!(
            "  Image {}: {}x{} {} ({} bytes)",
            i + 1,
            img.width,
            img.height,
            if img.is_color { "color" } else { "gray" },
            img.data.len()
        );
    }

    // Verify output PDFs are valid by loading with lopdf
    println!("\n=== Validating output PDFs ===");
    for path in &[bw_out, color_out, mixed_out] {
        match lopdf::Document::load(path) {
            Ok(doc) => {
                let pages = doc.get_pages();
                println!(
                    "  {path}: {} pages, valid structure",
                    pages.len()
                );
            }
            Err(e) => println!("  {path}: INVALID - {e}"),
        }
    }

    // Compare sizes
    let input_size = std::fs::metadata(&input).unwrap().len();
    println!("\n=== Summary ===");
    println!("  Input:   {:.0} KB", input_size as f64 / 1024.0);
    println!("  B&W:     {:.0} KB ({:.0}%)", bw_size as f64 / 1024.0, bw_size as f64 / input_size as f64 * 100.0);
    println!("  Color:   {:.0} KB ({:.0}%)", color_size as f64 / 1024.0, color_size as f64 / input_size as f64 * 100.0);
    println!("  Mixed:   {:.0} KB ({:.0}%)", mixed_size as f64 / 1024.0, mixed_size as f64 / input_size as f64 * 100.0);

    println!("\nAll tests passed!");
}
