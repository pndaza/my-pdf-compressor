pub mod convert;
pub mod extract;
pub mod pdf_writer;
pub mod types;

use std::sync::Mutex;
use types::*;

struct AppState {
    pdf_path: Mutex<Option<String>>,
}

#[tauri::command]
fn open_pdf(path: String, state: tauri::State<AppState>) -> Result<Vec<ImageInfo>, String> {
    let pages = extract::extract_all_images(&path)?;
    let mut infos = Vec::with_capacity(pages.len());

    for (index, (page_num, img)) in pages.into_iter().enumerate() {
        let thumbnail = extract::make_thumbnail(&img);
        let suggested_mode = if img.is_color { Mode::Color } else { Mode::Bw };

        let info = ImageInfo {
            index,
            page: page_num,
            width: img.width,
            height: img.height,
            is_color: img.is_color,
            original_size: img.data.len(),
            thumbnail,
            mode: suggested_mode,
        };
        infos.push(info);
    }

    *state.pdf_path.lock().unwrap() = Some(path);
    Ok(infos)
}

#[tauri::command]
async fn compress_pdf(
    choices: Vec<ImageChoice>,
    min_width: u32,
    uniform_page_size: bool,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<CompressResult, String> {
    let path = state
        .pdf_path
        .lock()
        .unwrap()
        .clone()
        .ok_or("No PDF loaded")?;

    let original_size = std::fs::metadata(&path)
        .map(|m| m.len())
        .unwrap_or(0);

    let pages = extract::extract_all_images(&path)?;

    // Auto-select most common page size if requested
    let uniform_size = if uniform_page_size {
        Some(find_most_common_page_size(&pages))
    } else {
        None
    };

    let choice_map: std::collections::HashMap<usize, Mode> =
        choices.into_iter().map(|c| (c.index, c.mode)).collect();

    let total = pages.len();
    let mut compressed_images: Vec<CompressedImage> = Vec::with_capacity(total);

    for (index, (_, img)) in pages.into_iter().enumerate() {
        let mode = choice_map.get(&index).copied().unwrap_or(if img.is_color {
            Mode::Color
        } else {
            Mode::Bw
        });

        let compressed = match mode {
            Mode::Bw => convert::convert_to_bw(&img, min_width)?,
            Mode::Color => convert::convert_to_color(&img, 30)?,
        };

        compressed_images.push(compressed);

        let _ = app.emit("compress-progress", serde_json::json!({
            "current": index + 1,
            "total": total,
        }));
    }

    let output_path = pick_save_path(&path)?;

    pdf_writer::write_pdf(&compressed_images, &output_path, uniform_size)?;

    let compressed_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(CompressResult {
        output_path,
        original_size,
        compressed_size,
        image_count: total,
    })
}

fn find_most_common_page_size(pages: &[(u32, types::DecodedImage)]) -> (f64, f64) {
    use std::collections::HashMap;
    let mut counts: HashMap<(i64, i64), usize> = HashMap::new();

    for (_, img) in pages {
        let dpi = if img.dpi > 0.0 { img.dpi } else { 200.0 };
        let w = (img.width as f64 / dpi * 72.0).round() as i64;
        let h = (img.height as f64 / dpi * 72.0).round() as i64;
        *counts.entry((w, h)).or_insert(0) += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|((w, h), count)| {
            eprintln!("Uniform page size: {w}x{h} pts ({count} pages)");
            (w as f64, h as f64)
        })
        .unwrap_or((612.0, 792.0))
}

fn pick_save_path(input_path: &str) -> Result<String, String> {
    let input = std::path::Path::new(input_path);
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let parent = input.parent().unwrap_or(std::path::Path::new("."));

    let mut counter = 0;
    loop {
        let suffix = if counter == 0 {
            "_compressed".to_string()
        } else {
            format!("_compressed_{counter}")
        };
        let candidate = parent.join(format!("{stem}{suffix}.pdf"));
        if !candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
        counter += 1;
    }
}

use tauri::Emitter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState {
            pdf_path: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![open_pdf, compress_pdf])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
