/* The Macros page: a grid of buttons drawn from macros.json. Saving the
   file shows up here within a couple of seconds. */

import { getMacros, runMacro } from "./api.js";

const $ = (id) => document.getElementById(id);
const IDLE = "Tap to run";
let shown = "";
let noteTimer;

/* Messages go in the shared title rail, where there is room to read them. */
function note(text, warn) {
  const n = $("macro-note");
  n.textContent = text;
  n.classList.toggle("silk--warn", warn);
  clearTimeout(noteTimer);
  if (warn) noteTimer = setTimeout(() => note(IDLE, false), 5000);
}

function part(cls, text) {
  const n = document.createElement("span");
  n.className = cls;
  n.textContent = text;
  return n;
}

function flash(btn, cls) {
  btn.classList.remove(cls);
  void btn.offsetWidth;
  btn.classList.add(cls);
}

/* The glass cap's lens. Near the rim, each pixel samples the screen
   from further in along the edge normal, so the screen's light bends
   out toward the edge the way thick glass bends it. Red and green
   carry the x and y offset around a neutral 128. */
function lensMap(w, h, r, rim) {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const ctx = c.getContext("2d");
  const img = ctx.createImageData(w, h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      // Depth inside the rounded rectangle, and the outward normal at the nearest edge.
      const px = x + 0.5 - w / 2;
      const py = y + 0.5 - h / 2;
      const qx = Math.abs(px) - (w / 2 - r);
      const qy = Math.abs(py) - (h / 2 - r);
      const ox = Math.max(qx, 0);
      const oy = Math.max(qy, 0);
      const len = Math.hypot(ox, oy);
      const depth = r - len - Math.min(Math.max(qx, qy), 0);
      let nx = len > 0 ? ox / len : qx > qy ? 1 : 0;
      let ny = len > 0 ? oy / len : qx > qy ? 0 : 1;
      nx *= Math.sign(px);
      ny *= Math.sign(py);
      const k = depth < rim ? (1 - Math.max(0, depth) / rim) ** 2 : 0;
      const i = (y * w + x) * 4;
      img.data[i] = 128 - nx * k * 127;
      img.data[i + 1] = 128 - ny * k * 127;
      img.data[i + 2] = 128;
      img.data[i + 3] = 255;
    }
  }
  ctx.putImageData(img, 0, 0);
  return c.toDataURL();
}

let lensSize = "";
function fitLens(key) {
  // The cap sits inside the key's border, so the map matches the padding box.
  const w = key.clientWidth;
  const h = key.clientHeight;
  const size = `${w}x${h}`;
  if (!w || !h || size === lensSize) return;
  lensSize = size;
  const css = getComputedStyle(key);
  const r = (parseFloat(css.borderTopLeftRadius) || 16) - (parseFloat(css.borderTopWidth) || 0);
  const map = $("glass-map");
  map.setAttribute("width", w);
  map.setAttribute("height", h);
  map.setAttribute("href", lensMap(w, h, r, 10));
}

async function fire(btn, index, label) {
  try {
    await runMacro(index, label);
    flash(btn, "is-fired");
  } catch (e) {
    flash(btn, "is-miss");
    note(String(e).replace(/^Error:\s*/, ""), true);
    refresh();
  }
}

function render(v) {
  // Only rebuild when the file actually changed, so a tap is never lost mid-redraw.
  const key = JSON.stringify(v);
  if (key === shown) return;
  shown = key;

  const grid = $("macro-grid");
  const empty = $("macro-empty");
  const problem = v.error ?? (v.buttons.length ? null : "No buttons in macros.json yet.");
  grid.hidden = Boolean(problem);
  empty.hidden = !problem;
  if (problem) {
    empty.replaceChildren(part("macro-empty__why", problem), part("macro-empty__path", v.path));
    return;
  }

  grid.style.setProperty("--cols", v.columns);
  // Built with DOM calls: labels and colours come from a file anyone can edit.
  grid.replaceChildren(...v.buttons.map((b, i) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "macro";
    if (b.color) btn.style.setProperty("--key", b.color);
    btn.append(part("macro__icon", b.icon ?? ""), part("macro__label", b.label));
    if (b.error) {
      btn.disabled = true;
      btn.append(part("macro__error", b.error));
    }
    btn.addEventListener("click", () => fire(btn, i, b.label));
    return btn;
  }));
  // Every key shares one grid cell size, so one map serves them all.
  if (grid.firstChild) fitLens(grid.firstChild);
}

async function refresh() {
  try {
    render(await getMacros());
  } catch (e) {
    note(String(e).replace(/^Error:\s*/, ""), true);
  }
}

export function wireMacros() {
  refresh();
  // Only while the page is in view; reading a small file is cheap, but not free.
  setInterval(() => {
    if (document.body.dataset.page === "macros") refresh();
  }, 2000);
}
