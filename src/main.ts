import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";

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
let isLoading = false;

const app = document.getElementById("app")!;

// Prevent browser default drag-and-drop (opening the file)
document.addEventListener("dragover", (e) => e.preventDefault());
document.addEventListener("drop", (e) => e.preventDefault());

// Tauri drag-and-drop
const webview = getCurrentWebview();
webview.onDragDropEvent((event) => {
  if (event.payload.type === "enter" || event.payload.type === "over") {
    if (!isLoading) {
      document.getElementById("drop-target")?.classList.add("drag-active");
      showDropOverlay();
    }
  } else if (event.payload.type === "leave") {
    document.getElementById("drop-target")?.classList.remove("drag-active");
    hideDropOverlay();
  } else if (event.payload.type === "drop") {
    document.getElementById("drop-target")?.classList.remove("drag-active");
    hideDropOverlay();
    const paths = (event.payload as { paths: string[] }).paths;
    const pdf = paths.find((p) => p.toLowerCase().endsWith(".pdf"));
    if (pdf) {
      loadPdf(pdf);
    } else if (paths.length > 0) {
      showError("Please drop a PDF file");
    }
  }
});

function render() {
  if (images.length === 0) {
    app.innerHTML = `
      <div class="toolbar">
        <h1>PDF Compress</h1>
      </div>
      <div class="empty-state">
        <div class="drop-target" id="drop-target">
          <div class="drop-icon">
            <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
              <polyline points="14 2 14 8 20 8"/>
              <line x1="12" y1="18" x2="12" y2="12"/>
              <polyline points="9 15 12 12 15 15"/>
            </svg>
          </div>
          <div class="drop-title">Drop a PDF here</div>
          <div class="drop-subtitle">or <span>click to browse</span></div>
          <div class="drop-info">
            <div class="info-row">
              <span class="info-label">B&amp;W pages</span>
              <span class="info-val">CCITT Group 4</span>
            </div>
            <div class="info-row">
              <span class="info-label">Color pages</span>
              <span class="info-val">JPEG 30%</span>
            </div>
            <div class="info-row">
              <span class="info-label">No re-encoding</span>
              <span class="info-val">Images embedded as-is</span>
            </div>
          </div>
        </div>
      </div>
    `;
    const target = document.getElementById("drop-target")!;
    target.onclick = openFile;
    return;
  }

  app.innerHTML = `
    <div class="toolbar">
      <button class="btn btn-icon" id="back-btn" title="Back">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="15 18 9 12 15 6"/>
        </svg>
      </button>
      <h1>PDF Compress</h1>
      <button class="btn btn-primary" id="compress-btn">Compress &amp; Save</button>
    </div>
    <div class="batch-controls">
      <span style="font-size:12px;color:var(--text-dim)">Set all:</span>
      <button class="btn btn-sm" id="all-bw">B&amp;W</button>
      <button class="btn btn-sm" id="all-color">Color</button>
      <button class="btn btn-sm" id="all-auto">Auto</button>
      <div class="control-sep"></div>
      <label class="select-label" for="enlarge-select">Enlarge small images</label>
      <button class="btn-help" id="enlarge-help" title="When to use enlarge?">?</button>
      <select id="enlarge-select" class="select-input">
        <option value="0">Off</option>
        <option value="1000">1000px</option>
        <option value="1500" selected>1500px</option>
        <option value="2000">2000px</option>
        <option value="3000">3000px</option>
      </select>
      <div class="control-sep"></div>
      <label class="checkbox-label">
        <input type="checkbox" id="uniform-checkbox" checked />
        <span>Uniform page size</span>
      </label>
    </div>
    <div class="content">
      <div class="image-grid" id="grid"></div>
    </div>
  `;

  document.getElementById("back-btn")!.onclick = () => {
    images = [];
    render();
  };
  document.getElementById("compress-btn")!.onclick = compress;
  document.getElementById("all-bw")!.onclick = () => setAllMode("bw");
  document.getElementById("all-color")!.onclick = () => setAllMode("color");
  document.getElementById("all-auto")!.onclick = setAuto;
  document.getElementById("enlarge-help")!.onclick = showEnlargeHelp;

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
  await loadPdf(selected);
}

async function loadPdf(path: string) {
  isLoading = true;
  showLoading("Extracting images...");

  try {
    images = await invoke<ImageInfo[]>("open_pdf", { path });
    render();
  } catch (e) {
    showError(String(e));
  } finally {
    isLoading = false;
    hideLoading();
  }
}

async function compress() {
  const btn = document.getElementById("compress-btn") as HTMLButtonElement;
  btn.disabled = true;

  const choices = images.map((img) => ({ index: img.index, mode: img.mode }));
  const minWidth = parseInt(
    (document.getElementById("enlarge-select") as HTMLSelectElement).value
  );
  const uniformPageSize = (document.getElementById("uniform-checkbox") as HTMLInputElement).checked;

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
    const result = await invoke<CompressResult>("compress_pdf", { choices, minWidth, uniformPageSize });
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
  hideLoading();
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

function hideLoading() {
  document.getElementById("loading-overlay")?.remove();
}

function showDropOverlay() {
  if (document.getElementById("drop-overlay")) return;
  const overlay = document.createElement("div");
  overlay.className = "drop-overlay";
  overlay.id = "drop-overlay";
  overlay.innerHTML = `
    <div class="drop-zone">
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
        <polyline points="17 8 12 3 7 8"/>
        <line x1="12" y1="3" x2="12" y2="15"/>
      </svg>
      <p>Drop PDF here</p>
    </div>
  `;
  document.body.appendChild(overlay);
}

function hideDropOverlay() {
  document.getElementById("drop-overlay")?.remove();
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

function showEnlargeHelp() {
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";

  const content: Record<string, { title: string; intro1: string; intro2: string; use: string; useItems: string[]; off: string; offItems: string[]; note: string }> = {
    en: {
      title: "Enlarge small images",
      intro1: "When B&amp;W pages come from low-resolution scans (under ~1500px wide), converting directly to 1-bit black &amp; white produces <b>jagged, pixelated edges</b>. Text looks blocky and hard to read.",
      intro2: "<b>Enable enlarge</b> to upsample the image with bicubic interpolation <i>before</i> thresholding. This creates smooth grayscale gradients at the edges, so the 1-bit output has cleaner, anti-aliased lines.",
      use: "When to use",
      useItems: [
        "Low-DPI scans or web-quality PDFs (images under 1500px wide)",
        "When B&amp;W text looks jagged or blocky in the output",
      ],
      off: "When to turn off",
      offItems: [
        "High-resolution scans (300+ DPI, 2000px+ wide)",
        "When minimizing file size is the priority",
      ],
      note: "Trade-off: enlarged images produce slightly larger G4 output because there are more pixels to encode, but the visual quality is significantly better for small sources.",
    },
    mm: {
      title: "ပုံအသေးစားများကို ချဲ့ထွင်ခြင်း",
      intro1: "အဖြူအမည် စာမျက်နှာများသည် အရွယ်အစားသေးငယ်သော (၁၅၀၀ ပီဇယ်အောက်) ပုံများမှ လာပါက၊ တိုက်ရိုက် အဖြူ/အမည် သို့ ပြောင်းလဲလိုက်သည့်အခါ <b>အစွန်းများ ထွက်ပေါ်လာပြီး</b> စာလုံးများ ထစ်ငန်းငန်း ဖြစ်သွားပါသည်။",
      intro2: "<b>ချဲ့ထွင်မှုကို ဖွင့်ထားခြင်း</b>ဖြင့် ပုံကို bicubic နည်းလမ်းဖြင့် ချဲ့ထွင်ပေးပြီး <i>ထို့နောက်</i> အဖြူ/အမည်သို့ ပြောင်းလဲပါသည်။ အစွန်းများ ပိုမို ချောမွေ့ပြီး စာလုံးများ ဖတ်ရလွယ်ကူစေပါသည်။",
      use: "အသုံးပြုသင့်သည့်အခါ",
      useItems: [
        "အရည်အသွေးနိမ့်သော စကင်(န်)များ (ပုံအကျယ် ၁၅၀၀ ပီဇယ်အောက်)",
        "အဖြူ/အမည် စာလုံးများ ထစ်ငန်းငန်း ဖြစ်နေသည့်အခါ",
      ],
      off: "ပိတ်သင့်သည့်အခါ",
      offItems: [
        "အရည်အသွေးမြင့် စကင်(န်)များ (၃၀၀+ DPI၊ ၂၀၀၀ ပီဇယ်အထက်)",
        "ဖိုင်အရွယ်အစား အသေးဆုံးဖြစ်ချင်သည့်အခါ",
      ],
      note: "သတိပြုရန် — ချဲ့ထွင်ထားသော ပုံများသည် G4 ဖိုင်အရွယ်အစား အနည်းငယ် ပိုကြီးစေသော်လည်း ပုံအရည်အသွေးမှာ သိသိသာသာ ပိုကောင်းစေပါသည်။",
    },
  };

  let lang = "en";

  function renderHelp() {
    const c = content[lang];
    const items = (items: string[]) => items.map((item) => `<li>${item}</li>`).join("");
    return `
      <div class="help-box">
        <div class="help-header">
          <h3>${c.title}</h3>
          <div class="lang-toggle">
            <button class="lang-btn ${lang === "en" ? "active" : ""}" data-lang="en">EN</button>
            <button class="lang-btn ${lang === "mm" ? "active" : ""}" data-lang="mm">မြန်မာ</button>
          </div>
        </div>
        <p>${c.intro1}</p>
        <p>${c.intro2}</p>
        <div class="help-tip">
          <b>${c.use}</b>
          <ul>${items(c.useItems)}</ul>
        </div>
        <div class="help-tip">
          <b>${c.off}</b>
          <ul>${items(c.offItems)}</ul>
        </div>
        <p class="help-note">${c.note}</p>
        <button class="btn btn-primary" id="close-help" style="margin-top:4px">Got it</button>
      </div>
    `;
  }

  overlay.innerHTML = renderHelp();
  app.appendChild(overlay);

  function bind() {
    overlay.querySelector("#close-help")!.addEventListener("click", () => overlay.remove());
    overlay.querySelectorAll(".lang-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        lang = (btn as HTMLElement).dataset.lang!;
        overlay.innerHTML = renderHelp();
        bind();
      });
    });
  }

  bind();
  overlay.onclick = (e) => { if (e.target === overlay) overlay.remove(); };
}

function showError(msg: string) {  const overlay = document.createElement("div");
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
