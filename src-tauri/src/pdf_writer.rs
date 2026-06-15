use crate::types::CompressedImage;
use std::io::Write;

pub fn write_pdf(
    images: &[CompressedImage],
    output_path: &str,
    uniform_size: Option<(f64, f64)>,
) -> Result<(), String> {
    let mut buf = Vec::new();
    write_pdf_bytes(images, &mut buf, uniform_size)?;
    std::fs::write(output_path, buf).map_err(|e| format!("Failed to write output: {e}"))?;
    Ok(())
}

fn write_pdf_bytes(
    images: &[CompressedImage],
    w: &mut Vec<u8>,
    uniform_size: Option<(f64, f64)>,
) -> Result<(), String> {
    // %PDF-1.4 header with binary comment
    w.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");

    let n = images.len() as u32;
    let mut offsets: Vec<usize> = Vec::new();

    // obj 1: Catalog -> Pages obj 2
    offsets.push(w.len());
    write!(w, "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n").unwrap();

    // obj 2: Pages
    offsets.push(w.len());
    write!(w, "2 0 obj\n<< /Type /Pages /Count {n} /Kids [").unwrap();
    for i in 0..n {
        let page_obj = 3 + 3 * i;
        write!(w, "{page_obj} 0 R ").unwrap();
    }
    write!(w, "] >>\nendobj\n").unwrap();

    for (i, img) in images.iter().enumerate() {
        let i = i as u32;
        let page_obj = 3 + 3 * i;
        let image_obj = 4 + 3 * i;
        let content_obj = 5 + 3 * i;

        // Page dimensions in points — use uniform size if provided
        let (page_w, page_h) = if let Some((uw, uh)) = uniform_size {
            (uw, uh)
        } else {
            let dpi = if img.dpi > 0.0 { img.dpi } else { 200.0 };
            (
                (img.width as f64 / dpi * 72.0).round(),
                (img.height as f64 / dpi * 72.0).round(),
            )
        };

        // Page object (no stream)
        offsets.push(w.len());
        write!(
            w,
            "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w} {page_h}] /Resources << /XObject << /Im0 {image_obj} 0 R >> >> /Contents {content_obj} 0 R >>\nendobj\n"
        ).unwrap();

        // Image XObject (stream object) - embed raw compressed data, NO re-encoding
        offsets.push(w.len());
        write!(
            w,
            "{image_obj} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} /BitsPerComponent {} /Filter /{}",
            img.width, img.height, img.color_space, img.bits_per_component, img.filter
        ).unwrap();

        if img.is_ccitt {
            // DecodeParms for CCITT Group 4: K=-1
            write!(
                w,
                " /DecodeParms << /K -1 /Columns {} /Rows {} >>",
                img.width, img.height
            ).unwrap();
        }

        write!(w, " /Length {} >>\nstream\n", img.data.len()).unwrap();
        w.extend_from_slice(&img.data);
        w.extend_from_slice(b"\nendstream\nendobj\n");

        // Content stream: draw image to fill page
        let content = format!("q\n{page_w} 0 0 {page_h} 0 0 cm\n/Im0 Do\nQ\n");
        offsets.push(w.len());
        write!(
            w,
            "{content_obj} 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            content.len(),
            content
        ).unwrap();
    }

    // Cross-reference table
    let xref_offset = w.len();
    let total_objects = 2 + 3 * n + 1; // +1 for obj 0 (free)
    w.extend_from_slice(b"xref\n");
    write!(w, "0 {total_objects}\n").unwrap();
    w.extend_from_slice(b"0000000000 65535 f \n");
    for &offset in &offsets {
        write!(w, "{:010} 00000 n \n", offset).unwrap();
    }

    // Trailer
    w.extend_from_slice(b"trailer\n");
    write!(w, "<< /Size {total_objects} /Root 1 0 R >>\n").unwrap();
    w.extend_from_slice(b"startxref\n");
    write!(w, "{xref_offset}\n").unwrap();
    w.extend_from_slice(b"%%EOF\n");

    Ok(())
}
