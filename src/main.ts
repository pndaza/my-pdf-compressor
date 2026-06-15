import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

interface ImageInfo {
  index: number;
  page: number;
  width: number;
  height: number;
  is_color: boolean;
  original_size: number;
  thumbnail: string;
  mode: "bw" | "color";
}

interface CompressResult {
  output_path: string;
  original_size: number;
  compressed_size: number;
  image_count: number;
}

let images: ImageInfo[] = [];

const app = document.getElementById("app")!;

function render() {
  if (images.length === 0) {
    app.innerHTML = `
      <div class="toolbar">
        <h1>PDF Compress</h1>
        <button class="btn btn-primary" id="open-btn">Open PDF</button>
      </div>
      <div class="empty-state">
        <p>Select a scanned PDF to compress</p>
        <button class="btn btn-primary" id="open-btn2">Choose File</button>
      </div>
    `;
    document.getElementById("open-btn")!.onclick = openFile;
    document.getElementById("open-btn2")!.onclick = openFile;
    return;
  }

  app.innerHTML = `
    <div class="toolbar">
      <h1>PDF Compress</h1>
      <button class="btn btn-sm" id="open-btn">Open PDF</button>
      <button class="btn btn-primary" id="compress-btn">Compress &amp; Save</button>
    </div>
    <div class="batch-controls">
      <span style="font-size:12px;color:var(--text-dim)">Set all:</span>
      <button class="btn btn-sm" id="all-bw">B&amp;W</button>
      <button class="btn btn-sm" id="all-color">Color</button>
      <button class="btn btn-sm" id="all-auto">Auto</button>
    </div>
    <div class="content">
      <div class="image-grid" id="grid"></div>
    </div>
  `;

  document.getElementById("open-btn")!.onclick = openFile;
  document.getElementById("compress-btn")!.onclick = compress;
  document.getElementById("all-bw")!.onclick = () => setAllMode("bw");
  document.getElementById("all-color")!.onclick = () => setAllMode("color");
  document.getElementById("all-auto")!.onclick = setAuto;

  renderGrid();
}

function renderGrid() {
  const grid = document.getElementById("grid")!;
  grid.innerHTML = images
    .map(
      (img) => `
    <div class="image-card">
      <img class="thumb" src="${img.thumbnail}" alt="Page ${img.page}" />
      <div class="info">
        <span class="page-label">Page ${img.page} ${img.is_color ? "(color)" : "(gray)"}</span>
        <span class="dims">${img.width}&times;${img.height}</span>
        <div class="mode-toggle" data-index="${img.index}">
          <button class="bw ${img.mode === "bw" ? "active" : ""}">B&amp;W</button>
          <button class="color ${img.mode === "color" ? "active" : ""}">Color</button>
        </div>
      </div>
    </div>
  `
    )
    .join("");

  grid.querySelectorAll(".mode-toggle").forEach((el) => {
    const idx = parseInt(el.getAttribute("data-index")!);
    el.querySelectorAll("button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const mode = btn.classList.contains("bw") ? "bw" : "color";
        images[idx].mode = mode;
        renderGrid();
      });
    });
  });
}

function setAllMode(mode: "bw" | "color") {
  images.forEach((img) => (img.mode = mode));
  renderGrid();
}

function setAuto() {
  images.forEach((img) => (img.mode = img.is_color ? "color" : "bw"));
  renderGrid();
}

async function openFile() {
  const selected = await open({
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });

  if (!selected) return;

  showLoading("Extracting images...");

  try {
    images = await invoke<ImageInfo[]>("open_pdf", { path: selected });
    render();
  } catch (e) {
    showError(String(e));
  }
}

async function compress() {
  const btn = document.getElementById("compress-btn") as HTMLButtonElement;
  btn.disabled = true;

  const choices = images.map((img) => ({ index: img.index, mode: img.mode }));

  // Show progress overlay
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="progress-box">
      <h3>Compressing...</h3>
      <div class="progress-bar"><div class="progress-bar-fill" style="width:0%"></div></div>
      <div class="progress-text">0 / ${images.length}</div>
    </div>
  `;
  app.appendChild(overlay);

  const unlisten = await listen<{ current: number; total: number }>(
    "compress-progress",
    (event) => {
      const { current, total } = event.payload;
      const pct = (current / total) * 100;
      overlay.querySelector(".progress-bar-fill")!.setAttribute("style", `width:${pct}%`);
      overlay.querySelector(".progress-text")!.textContent = `${current} / ${total}`;
    }
  );

  try {
    const result = await invoke<CompressResult>("compress_pdf", { choices });
    unlisten();
    overlay.remove();
    showResult(result);
  } catch (e) {
    unlisten();
    overlay.remove();
    showError(String(e));
  }
}

function showLoading(msg: string) {
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.id = "loading-overlay";
  overlay.innerHTML = `
    <div class="progress-box">
      <h3>${msg}</h3>
    </div>
  `;
  app.appendChild(overlay);
}

function showResult(result: CompressResult) {
  const ratio = ((1 - result.compressed_size / result.original_size) * 100).toFixed(1);
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="result-box">
      <h3>Saved ${ratio}%</h3>
      <div class="result-stats">
        <div class="result-stat">
          <span class="label">Original</span>
          <span class="value">${formatSize(result.original_size)}</span>
        </div>
        <div class="result-stat">
          <span class="label">Compressed</span>
          <span class="value">${formatSize(result.compressed_size)}</span>
        </div>
        <div class="result-stat">
          <span class="label">Pages</span>
          <span class="value">${result.image_count}</span>
        </div>
      </div>
      <p style="margin-top:16px;font-size:12px;color:var(--text-dim);word-break:break-all">${result.output_path}</p>
      <button class="btn btn-primary" style="margin-top:16px" id="close-result">Done</button>
    </div>
  `;
  app.appendChild(overlay);
  document.getElementById("close-result")!.onclick = () => overlay.remove();
}

function showError(msg: string) {
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="result-box">
      <h3 style="color:var(--accent)">Error</h3>
      <p style="font-size:13px;color:var(--text-dim);margin-top:8px">${msg}</p>
      <button class="btn btn-primary" style="margin-top:16px" id="close-error">OK</button>
    </div>
  `;
  app.appendChild(overlay);
  document.getElementById("close-error")!.onclick = () => overlay.remove();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

render();
