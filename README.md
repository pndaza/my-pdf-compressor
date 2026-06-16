# My PDF Compressor

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
npm run tauri dev      # development
npm run tauri build    # production bundle
```

### Core pipeline

| Module | Responsibility |
|--------|---------------|
| `extract.rs` | Extract embedded image XObjects from PDF (lopdf) |
| `convert.rs` | B&W: threshold + CCITT G4 (fax crate). Color: JPEG (image crate) |
| `pdf_writer.rs` | Assemble new PDF from re-encoded image data (hand-writes PDF syntax) |

### CLI tools

```bash
cargo run --bin test-pipeline -- /path/to/file.pdf    # test pipeline
cargo run --bin compare_images                        # compare modes
```
