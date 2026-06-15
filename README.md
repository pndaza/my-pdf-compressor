# PDF Compress

Cross-platform desktop app to compress scanned PDFs. Built with Tauri 2 + Rust.

## User

1. Drop a PDF (or click to browse)
2. Toggle each page between **B&W** (CCITT Group 4) or **Color** (JPEG 30%)
3. Optionally enable **Enlarge** for small images and **Uniform page size**
4. Click **Compress & Save**

Output is saved next to the original as `*_compressed.pdf`.

## Developer

```bash
npm install
cargo tauri dev      # development
cargo tauri build    # production bundle
```

### Architecture

| File | Responsibility |
|------|---------------|
| `src-tauri/src/extract.rs` | Extract embedded image XObjects from PDF (lopdf) |
| `src-tauri/src/convert.rs` | B&W: threshold + CCITT G4 (fax crate). Color: JPEG (image crate) |
| `src-tauri/src/pdf_writer.rs` | Write PDF embedding raw compressed data, no re-encoding |
| `src-tauri/src/lib.rs` | Tauri commands + state |
| `src/main.ts` | UI: drag-drop, image grid, controls |
| `src/style.css` | Styling |

### CLI tools

```bash
cargo run --bin test-pipeline -- /path/to/file.pdf    # test pipeline
cargo run --bin compare_images                        # compare modes
```
