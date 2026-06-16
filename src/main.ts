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

// ── i18n ──────────────────────────────────────────────────────────────────
type Lang = "en" | "mm";

const LANG_KEY = "pdf-compress-lang";

const en = {
  appTitle: "PDF Compress",
  back: "Back",
  compressBtn: "Compress & Save",
  dropTitle: "Drop a PDF here",
  dropOr: "or",
  dropBrowse: "click to browse",
  tagline: "Lossless for B&W · JPEG for color · No re-encoding",
  setAll: "Set all:",
  bw: "B&W",
  color: "Color",
  auto: "Auto",
  enlargeLabel: "Enlarge small images",
  enlargeHelpTitle: "When to use enlarge?",
  enlargeOff: "Off",
  uniformLabel: "Uniform page size",
  page: "Page",
  colorTag: "(color)",
  grayTag: "(gray)",
  extracting: "Extracting images...",
  compressing: "Compressing...",
  saved: "Saved",
  original: "Original",
  compressed: "Compressed",
  pages: "Pages",
  done: "Done",
  pleaseDropPdf: "Please drop a PDF file",
  error: "Error",
  ok: "OK",
  dropPdfHere: "Drop PDF here",
  gotIt: "Got it",
  howCompressWorks: "How compression works",
  compressInfo: {
    title: "How compression works",
    intro:
      "Most PDF compressors take a one-size-fits-all approach that hurts quality. PDF Compress picks the right codec for each image instead.",
    mostTitle: "How most compressors do it",
    mostItems: [
      "Apply one lossy JPEG pass to every page — even crisp B&W text.",
      "B&W scans get converted to 8-bit JPEG, adding blurry 'mosquito noise' around letters while barely shrinking the file.",
      "A single quality/DPI setting can't suit both sharp text and color photos.",
      "Aggressive presets (72–150 DPI) make scanned text fuzzy and hard to read.",
    ],
    oursTitle: "How PDF Compress does it",
    items: [
      { label: "B&W images", val: "CCITT Group 4 — 1-bit fax encoding. Tiny files, crisp text." },
      { label: "Color images", val: "JPEG at 30% quality. Small size, fine for on-screen viewing." },
      { label: "Images only", val: "All images are re-encoded. Text, annotations, and vector graphics from the original are removed." },
    ],
    note: "The result is a dramatically smaller PDF with minimal visible quality loss — ideal for scans and image-heavy documents.",
  },
  enlargeHelp: {
    title: "Enlarge small images",
    intro1:
      "When B&W pages come from low-resolution scans (under ~1500px wide), converting directly to 1-bit black & white produces <b>jagged, pixelated edges</b>. Text looks blocky and hard to read.",
    intro2:
      "<b>Enable enlarge</b> to upsample the image with bicubic interpolation <i>before</i> thresholding. This creates smooth grayscale gradients at the edges, so the 1-bit output has cleaner, anti-aliased lines.",
    use: "When to use",
    useItems: [
      "Low-DPI scans or web-quality PDFs (images under 1500px wide)",
      "When B&W text looks jagged or blocky in the output",
    ],
    off: "When to turn off",
    offItems: [
      "High-resolution scans (300+ DPI, 2000px+ wide)",
      "When minimizing file size is the priority",
    ],
    note: "Trade-off: enlarged images produce slightly larger G4 output because there are more pixels to encode, but the visual quality is significantly better for small sources.",
  },
};

const mm: typeof en = {
  appTitle: "PDF Compress",
  back: "နောက်သို့",
  compressBtn: "ချုံ့ပြီး သိမ်းဆည်းရန်",
  dropTitle: "PDF ဖိုင်ကို ဤနေရာတွင် ဆွဲချပါ",
  dropOr: "သို့မဟုတ်",
  dropBrowse: "နှိပ်၍ ရွေးချယ်ရန်",
  tagline: "ဖြူ/မဲအတွက် ဆုံးရှုံးမှုမရှိ · အရောင်အတွက် JPEG · ပြန်လည် encode မလုပ်ပါ",
  setAll: "အားလုံးသတ်မှတ်ရန်:",
  bw: "ဖြူ/မဲ",
  color: "အရောင်",
  auto: "အလိုအလျောက်",
  enlargeLabel: "ပုံအသေးစားများကို ချဲ့ထွင်ခြင်း",
  enlargeHelpTitle: "ပုံချဲ့ထွင်မှုကို ဘယ်အချိန်သုံးမလဲ?",
  enlargeOff: "ပိတ်",
  uniformLabel: "စာမျက်နှာအရွယ်အစား တူညီစေရန်",
  page: "စာမျက်နှာ",
  colorTag: "(အရောင်)",
  grayTag: "(မီးခိုးရောင်)",
  extracting: "ပုံများ ထုတ်ယူနေသည်...",
  compressing: "ချုံ့နေသည်...",
  saved: "သိမ်းဆည်းပြီး",
  original: "မူရင်း",
  compressed: "ချုံ့ပြီး",
  pages: "စာမျက်နှာ",
  done: "ပြီးပါပြီ",
  pleaseDropPdf: "ကျေးဇူးပြု၍ PDF ဖိုင်ကို ဆွဲချပါ",
  error: "အမှား",
  ok: "OK",
  dropPdfHere: "PDF ကို ဤနေရာတွင် ဆွဲချပါ",
  gotIt: "နားလည်ပါပြီ",
  howCompressWorks: "ဖိုင်ချုံ့စနစ် လုပ်ဆောင်ပုံ",
  compressInfo: {
    title: "ဖိုင်ချုံ့စနစ် လုပ်ဆောင်ပုံ",
    intro:
      "ပုံမှန် PDF compressor အများစုသည် အရည်အသွေးကို ထိခိုက်စေသည့် တစ်ပြေးညီနည်းလမ်းကိုသာ သုံးကြသည်။ PDF Compress ကမူ ပုံတစ်ခုချင်းအလိုက် အသင့်တော်ဆုံး codec ကို စနစ်တကျ ရွေးချယ်ပေးပါသည်။",
    mostTitle: "အခြား compressor များ လုပ်ဆောင်ပုံ",
    mostItems: [
      "ကြည်လင်ပြတ်သားသည့် ဖြူ/မဲစာသားများ အပါအဝင် စာမျက်နှာအားလုံးကို lossy JPEG pass တစ်ခုတည်းဖြင့်သာ ပြောင်းလဲပစ်သည်။",
      "ဖြူ/မဲ စကင်ဖတ်ထားသည်များကို 8-bit JPEG အဖြစ် ပြောင်းလဲလိုက်သဖြင့် ဖိုင်အရွယ်အစား သိသိသာသာ မလျော့ကျဘဲ စာလုံးဘေးပတ်ပတ်လည်တွင် ဝါးတားတား အစက်အပြောက်များ (mosquito noise) ဖြစ်ပေါ်စေသည်။",
      "တစ်ခုတည်းသော Quality/DPI သတ်မှတ်ချက်သည် ပြတ်သားသော စာသားများနှင့် အရောင်ပါသော ဓာတ်ပုံနှစ်မျိုးလုံးအတွက် အဆင်မပြေနိုင်ပါ။",
      "အလွန်အကျွံချုံ့ပစ်သည့် အသင့်ပြင်ဆက်တင်များ (72–150 DPI) ကြောင့် စကင်ဖတ်ထားသော စာသားများ မှုန်ဝါးသွားပြီး ဖတ်ရခက်ခဲစေသည်။",
    ],
    oursTitle: "PDF Compress လုပ်ဆောင်ပုံ",
    items: [
      { label: "ဖြူ/မဲ ပုံများ", val: "CCITT Group 4 — 1-bit fax encoding သုံးသည်။ ဖိုင်အရွယ်အစား အလွန်သေးငယ်ပြီး စာသားများ ကြည်လင်ပြတ်သားသည်။" },
      { label: "အရောင်ပါ ပုံများ", val: "JPEG ကို Quality 30% ဖြင့် သုံးသည်။ ဖိုင်အရွယ်အစား သေးငယ်ပြီး မျက်နှာပြင်ပေါ်တွင် ကြည့်ရှုရန် အဆင်ပြေသည်။" },
      { label: "ပုံများသာ ထိန်းသိမ်းခြင်း", val: "ပုံအားလုံးကို ပြန်လည် encode လုပ်သည်။ မူရင်းရှိ စာသားများ၊ မှတ်ချက်များနှင့် vector ဂရပ်ဖစ်များ ဖယ်ရှားခံရပါသည်။" },
    ],
    note: "ရလဒ်အနေဖြင့် အရည်အသွေးကျဆင်းမှုကို မသိသာစေဘဲ ဖိုင်အရွယ်အစားကို သိသိသာသာ သေးငယ်သွားစေသဖြင့် စကင်ဖတ်ထားသော စာရွက်စာတမ်းများနှင့် ပုံအများအပြားပါဝင်သည့် ဖိုင်များအတွက် အသင့်တော်ဆုံး ဖြစ်သည်။",
  },
  enlargeHelp: {
    title: "ပုံအသေးစားများကို ချဲ့ထွင်ခြင်း",
    intro1:
      "အဖြူအမည်း စာမျက်နှာများသည် အရွယ်အစားသေးငယ်သော (အကျယ် ၁၅၀၀ ပီဇယ်အောက်) စကင်ပုံများဖြစ်ပါက၊ 1-bit ဖြူ/မဲအဖြစ် တိုက်ရိုက်ပြောင်းလဲသည့်အခါ <b>အစွန်းများ စောင်းထစ်ပြီး ပစ်ဇယ်ကွဲခြင်းများ</b> ဖြစ်ပေါ်တတ်သည်။ ထို့ကြောင့် စာသားများမှာ အကွက်လိုက်ဖြစ်ပြီး ဖတ်ရခက်ခဲစေသည်။",
    intro2:
      "<b>ပုံချဲ့ထွင်မှုကို ဖွင့်ထားပါက</b> ဖြူ/မဲအဖြစ် မပြောင်းလဲမီ ပုံကို bicubic interpolation နည်းလမ်းဖြင့် Resolution မြှင့်တင်ပေးမည် ဖြစ်သည်။ ၎င်းသည် စာလုံးအစွန်းများကို မီးခိုးရောင်ဖြန့်ခွဲမှုဖြင့် ချောမွေ့သွားစေသည့်အတွက် Output ထွက်လာသည့် 1-bit ဖိုင်တွင် ပိုမိုသန့်ရှင်းပြီး ကြည်လင်သော စာသားလိုင်းများကို ရရှိစေပါသည်။",
    use: "မည်သည့်အချိန်တွင် အသုံးပြုရမည်နည်း",
    useItems: [
      "DPI နိမ့်သော စကင်ဖတ်ချက်များ သို့မဟုတ် Web-quality PDF များ (ပုံအကျယ် ၁၅၀၀ ပီဇယ်အောက်)",
      "ရလဒ်ဖိုင်တွင် ဖြူ/မဲစာသားများ စောင်းထစ်ပြီး အကွက်လိုက် ဖြစ်နေသည့်အခါ",
    ],
    off: "မည်သည့်အချိန်တွင် ပိတ်ထားရမည်နည်း",
    offItems: [
      "Resolution မြင့်မားသော စကင်ဖတ်ချက်များ (300+ DPI သို့မဟုတ် ၂၀၀၀ ပီဇယ်အထက်)",
      "ဖိုင်အရွယ်အစား အသေးဆုံးဖြစ်ရန်သာ အဓိကဦးစားပေးလိုသည့်အခါ",
    ],
    note: "အပေးအယူအနေဖြင့် - ပုံကို ချဲ့ထွင်လိုက်ပါက encode လုပ်ရန် ပစ်ဇယ်အရေအတွက် ပိုများလာသောကြောင့် G4 ဖိုင်အရွယ်အစား အနည်းငယ် ပိုကြီးလာနိုင်သော်လည်း မူရင်း Resolution နိမ့်သော ဖိုင်များအတွက် Visual Quality ကို သိသိသာသာ ပိုမိုကောင်းမွန်စေပါသည်။",
  },
};

const translations: Record<Lang, typeof en> = { en, mm };

let lang: Lang = (localStorage.getItem(LANG_KEY) as Lang) || "en";

function tr(): typeof en {
  return translations[lang];
}

function setLang(l: Lang) {
  if (l === lang) return;
  lang = l;
  localStorage.setItem(LANG_KEY, l);
  document.documentElement.lang = l === "mm" ? "my" : "en";
  render();
}

function langToggleHTML() {
  return `
    <div class="lang-toggle" id="global-lang-toggle">
      <button class="lang-btn ${lang === "en" ? "active" : ""}" data-lang="en">EN</button>
      <button class="lang-btn ${lang === "mm" ? "active" : ""}" data-lang="mm">မြန်မာ</button>
    </div>
  `;
}

function bindLangToggle() {
  document
    .querySelectorAll<HTMLButtonElement>("#global-lang-toggle .lang-btn")
    .forEach((btn) => {
      btn.addEventListener("click", () => setLang(btn.dataset.lang as Lang));
    });
}

// ── App state ─────────────────────────────────────────────────────────────
let images: ImageInfo[] = [];
let isLoading = false;

const app = document.getElementById("app")!;
document.documentElement.lang = lang === "mm" ? "my" : "en";

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
      showError(tr().pleaseDropPdf);
    }
  }
});

function render() {
  const t = tr();
  if (images.length === 0) {
    app.innerHTML = `
      <div class="toolbar">
        <h1>${t.appTitle}</h1>
        ${langToggleHTML()}
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
          <div class="drop-title">${t.dropTitle}</div>
          <div class="drop-subtitle">${t.dropOr} <span>${t.dropBrowse}</span></div>
        </div>
      </div>
      <footer class="footer">
        <button class="footer-info-btn" id="compress-info-btn">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"/>
            <line x1="12" y1="16" x2="12" y2="12"/>
            <line x1="12" y1="8" x2="12.01" y2="8"/>
          </svg>
          ${t.howCompressWorks}
        </button>
      </footer>
    `;
    bindLangToggle();
    const target = document.getElementById("drop-target")!;
    target.onclick = openFile;
    document.getElementById("compress-info-btn")!.onclick = showCompressInfo;
    return;
  }

  app.innerHTML = `
    <div class="toolbar">
      <button class="btn btn-icon" id="back-btn" title="${t.back}">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="15 18 9 12 15 6"/>
        </svg>
      </button>
      <h1>${t.appTitle}</h1>
      ${langToggleHTML()}
      <button class="btn btn-primary" id="compress-btn">${t.compressBtn}</button>
    </div>
    <div class="batch-controls">
      <span style="font-size:12px;color:var(--text-dim)">${t.setAll}</span>
      <button class="btn btn-sm" id="all-bw">${t.bw}</button>
      <button class="btn btn-sm" id="all-color">${t.color}</button>
      <button class="btn btn-sm" id="all-auto">${t.auto}</button>
      <div class="control-sep"></div>
      <label class="select-label" for="enlarge-select">${t.enlargeLabel}</label>
      <button class="btn-help" id="enlarge-help" title="${t.enlargeHelpTitle}">?</button>
      <select id="enlarge-select" class="select-input">
        <option value="0">${t.enlargeOff}</option>
        <option value="1000">1000px</option>
        <option value="1500" selected>1500px</option>
        <option value="2000">2000px</option>
        <option value="3000">3000px</option>
      </select>
      <div class="control-sep"></div>
      <label class="checkbox-label">
        <input type="checkbox" id="uniform-checkbox" checked />
        <span>${t.uniformLabel}</span>
      </label>
    </div>
    <div class="content">
      <div class="image-grid" id="grid"></div>
    </div>
  `;

  bindLangToggle();
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
  const t = tr();
  const grid = document.getElementById("grid")!;
  grid.innerHTML = images
    .map(
      (img) => `
    <div class="image-card">
      <img class="thumb" src="${img.thumbnail}" alt="${t.page} ${img.page}" />
      <div class="info">
        <span class="page-label">${t.page} ${img.page} ${img.is_color ? t.colorTag : t.grayTag}</span>
        <span class="dims">${img.width}&times;${img.height}</span>
        <div class="mode-toggle" data-index="${img.index}">
          <button class="bw ${img.mode === "bw" ? "active" : ""}">${t.bw}</button>
          <button class="color ${img.mode === "color" ? "active" : ""}">${t.color}</button>
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
  showLoading(tr().extracting);

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
  const t = tr();
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
      <h3>${t.compressing}</h3>
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
      <p>${tr().dropPdfHere}</p>
    </div>
  `;
  document.body.appendChild(overlay);
}

function hideDropOverlay() {
  document.getElementById("drop-overlay")?.remove();
}

function showResult(result: CompressResult) {
  const t = tr();
  const ratio = ((1 - result.compressed_size / result.original_size) * 100).toFixed(1);
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="result-box">
      <h3>${t.saved} ${ratio}%</h3>
      <div class="result-stats">
        <div class="result-stat">
          <span class="label">${t.original}</span>
          <span class="value">${formatSize(result.original_size)}</span>
        </div>
        <div class="result-stat">
          <span class="label">${t.compressed}</span>
          <span class="value">${formatSize(result.compressed_size)}</span>
        </div>
        <div class="result-stat">
          <span class="label">${t.pages}</span>
          <span class="value">${result.image_count}</span>
        </div>
      </div>
      <p style="margin-top:16px;font-size:12px;color:var(--text-dim);word-break:break-all">${result.output_path}</p>
      <button class="btn btn-primary" style="margin-top:16px" id="close-result">${t.done}</button>
    </div>
  `;
  app.appendChild(overlay);
  document.getElementById("close-result")!.onclick = () => overlay.remove();
}

function showCompressInfo() {
  const t = tr();
  const c = t.compressInfo;
  const rows = c.items
    .map(
      (it) => `
      <div class="info-tip-row">
        <span class="info-tip-label">${it.label}</span>
        <span class="info-tip-val">${it.val}</span>
      </div>`
    )
    .join("");
  const mostBullets = c.mostItems.map((item) => `<li>${item}</li>`).join("");
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="help-box">
      <div class="help-header">
        <h3>${c.title}</h3>
      </div>
      <p>${c.intro}</p>
      <div class="help-tip help-tip-warn">
        <b>${c.mostTitle}</b>
        <ul>${mostBullets}</ul>
      </div>
      <div class="help-tip">
        <b>${c.oursTitle}</b>
        ${rows}
      </div>
      <p class="help-note">${c.note}</p>
      <button class="btn btn-primary" id="close-info" style="margin-top:4px">${t.gotIt}</button>
    </div>
  `;
  app.appendChild(overlay);
  overlay.querySelector("#close-info")!.addEventListener("click", () => overlay.remove());
  overlay.onclick = (e) => { if (e.target === overlay) overlay.remove(); };
}

function showEnlargeHelp() {
  const t = tr();
  const h = t.enlargeHelp;
  const items = (arr: string[]) => arr.map((item) => `<li>${item}</li>`).join("");
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="help-box">
      <div class="help-header">
        <h3>${h.title}</h3>
      </div>
      <p>${h.intro1}</p>
      <p>${h.intro2}</p>
      <div class="help-tip">
        <b>${h.use}</b>
        <ul>${items(h.useItems)}</ul>
      </div>
      <div class="help-tip">
        <b>${h.off}</b>
        <ul>${items(h.offItems)}</ul>
      </div>
      <p class="help-note">${h.note}</p>
      <button class="btn btn-primary" id="close-help" style="margin-top:4px">${t.gotIt}</button>
    </div>
  `;
  app.appendChild(overlay);
  overlay.querySelector("#close-help")!.addEventListener("click", () => overlay.remove());
  overlay.onclick = (e) => { if (e.target === overlay) overlay.remove(); };
}

function showError(msg: string) {
  const t = tr();
  const overlay = document.createElement("div");
  overlay.className = "progress-overlay";
  overlay.innerHTML = `
    <div class="result-box">
      <h3 style="color:var(--accent)">${t.error}</h3>
      <p style="font-size:13px;color:var(--text-dim);margin-top:8px">${msg}</p>
      <button class="btn btn-primary" style="margin-top:16px" id="close-error">${t.ok}</button>
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
