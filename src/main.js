import { snapshot } from "./data.js";

const NS = "http://www.w3.org/2000/svg";
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

const svg = document.getElementById("chart");
const paper = svg.parentElement;
const penEl = document.getElementById("pen");

/* Vertical headroom above the limit line, so the red band has
   somewhere to live. 100% sits at 100/112 of the plot height. */
const Y_MAX = 112;
const PAD = { left: 34, right: 26, top: 8, bottom: 19 };

const hidden = new Set();
let revealed = false;

/* Presentation attributes cannot resolve var(), so every paint
   property goes through the style declaration instead. */
const PAINT = new Set([
  "fill", "stroke", "opacity", "stroke-width", "stroke-dasharray",
  "stroke-linejoin", "stroke-linecap", "shape-rendering",
  "font-family", "font-size", "font-weight", "letter-spacing",
]);

function el(name, attrs = {}, text) {
  const node = document.createElementNS(NS, name);
  for (const [k, v] of Object.entries(attrs)) {
    if (PAINT.has(k)) {
      node.style.setProperty(k, k === "font-size" && typeof v === "number" ? `${v}px` : String(v));
    } else {
      node.setAttribute(k, v);
    }
  }
  if (text != null) node.textContent = text;
  return node;
}

function drawChart() {
  const { width, height } = paper.getBoundingClientRect();
  const W = Math.round(width);
  const H = Math.round(height);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.replaceChildren();

  const x0 = PAD.left;
  const x1 = W - PAD.right;
  const y0 = PAD.top;
  const y1 = H - PAD.bottom;
  const n = snapshot.series[0].values.length;

  const x = (i) => x0 + (i / (n - 1)) * (x1 - x0);
  const y = (v) => y1 - (v / Y_MAX) * (y1 - y0);

  /* --- printed scale ------------------------------------- */
  const defs = el("defs");
  const hatch = el("pattern", {
    id: "hatch", width: 6, height: 6,
    patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)",
  });
  hatch.append(el("line", {
    x1: 0, y1: 0, x2: 0, y2: 6,
    stroke: "var(--grid)", "stroke-width": 1, opacity: 0.22,
  }));
  const clip = el("clipPath", { id: "feed" });
  const wipe = el("rect", { x: x0 - 1, y: 0, width: 0, height: H });
  clip.append(wipe);
  defs.append(hatch, clip);
  svg.append(defs);

  const printed = el("g", { "shape-rendering": "crispEdges" });

  for (let k = 0; k <= 20; k++) {
    const gx = x(((n - 1) * k) / 20);
    printed.append(el("line", {
      x1: gx, y1: y0, x2: gx, y2: y1,
      stroke: "var(--grid)", "stroke-width": 1,
      opacity: k % 4 === 0 ? 0.26 : 0.11,
    }));
  }
  for (let v = 0; v <= 110; v += 10) {
    printed.append(el("line", {
      x1: x0, y1: y(v), x2: x1, y2: y(v),
      stroke: "var(--grid)", "stroke-width": 1,
      opacity: v % 25 === 0 ? 0.26 : 0.11,
    }));
  }
  svg.append(printed);

  /* --- over-range band ----------------------------------- */
  const band = el("g");
  band.append(el("rect", { x: x0, y: y0, width: x1 - x0, height: y(100) - y0, fill: "url(#hatch)" }));
  band.append(el("line", {
    x1: x0, y1: y(100), x2: x1, y2: y(100),
    stroke: "var(--grid)", "stroke-width": 1.3, opacity: 0.85,
  }));
  band.append(el("text", {
    x: x0 + 5, y: y(100) - 5,
    fill: "var(--grid)", opacity: 0.8,
    "font-family": "var(--font-silk)", "font-size": 7.5,
    "font-weight": 600, "letter-spacing": "0.16em",
  }, "LIMIT"));
  svg.append(band);

  /* --- scale numerals ------------------------------------ */
  const marks = el("g", {
    fill: "var(--grid)", opacity: 0.72,
    "font-family": "var(--font-silk)", "font-size": 7.5, "font-weight": 500,
  });
  for (const v of [0, 25, 50, 75, 100]) {
    marks.append(el("text", { x: x0 - 7, y: y(v) + 2.6, "text-anchor": "end" }, String(v)));
  }
  const hours = snapshot.windowHours;
  for (let h = hours; h >= 0; h--) {
    const i = (n - 1) * (1 - h / hours);
    const isNow = h === 0;
    marks.append(el("text", {
      x: isNow ? x1 : x(i),
      y: y1 + 12,
      "text-anchor": isNow ? "end" : h === hours ? "start" : "middle",
      "font-weight": isNow ? 700 : 500,
    }, isNow ? "NOW" : `−${h}H`));
  }
  svg.append(marks);

  /* --- the pens ------------------------------------------ */
  const ink = el("g", { "clip-path": "url(#feed)" });
  const base = new Array(n).fill(0);

  /* Graphite sits at the baseline so it never fuses with the
     graphite total pen riding the top of the stack. */
  for (const ch of [...snapshot.series].reverse()) {
    const top = base.map((b, i) => b + ch.values[i]);
    if (!hidden.has(ch.id)) {
      const up = top.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`);
      const down = base.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).reverse();
      ink.append(el("polygon", {
        points: [...up, ...down].join(" "),
        fill: ch.ink, opacity: ch.wash,
      }));
      /* each channel signs its own boundary at full ink */
      ink.append(el("polyline", {
        points: up.join(" "),
        fill: "none", stroke: ch.ink, "stroke-width": 1.1,
        "stroke-linejoin": "round", opacity: 0.85,
      }));
    }
    for (let i = 0; i < n; i++) base[i] = top[i];
  }

  /* the total is its own pen, and it always writes */
  ink.append(el("polyline", {
    points: base.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" "),
    fill: "none", stroke: "var(--ink-graphite)",
    "stroke-width": 1.7, "stroke-linejoin": "round", "stroke-linecap": "round",
  }));

  /* the recorder annotates its own high-water mark */
  const pk = base.reduce((best, v, i) => (v > base[best] ? i : best), 0);
  if (pk < n - 4) {
    const py = y(base[pk]);
    ink.append(el("line", {
      x1: x(pk), y1: py - 3, x2: x(pk), y2: py - 9,
      stroke: "var(--ink-graphite)", "stroke-width": 1, opacity: 0.7,
    }));
    ink.append(el("text", {
      x: x(pk), y: py - 13, "text-anchor": "middle",
      fill: "var(--ink-graphite)", opacity: 0.75,
      "font-family": "var(--font-silk)", "font-size": 7,
      "font-weight": 600, "letter-spacing": "0.13em",
    }, `PEAK ${base[pk].toFixed(1)}`));
  }

  const now = base[n - 1];
  ink.append(el("line", {
    x1: x(n - 1), y1: y(now), x2: x1, y2: y(now),
    stroke: "var(--ink-graphite)", "stroke-width": 1,
    "stroke-dasharray": "1 3", opacity: 0.55,
  }));
  svg.append(ink);

  /* the pen's rail at NOW */
  svg.append(el("line", {
    x1: x1, y1: y0, x2: x1, y2: y1,
    stroke: "var(--ink-graphite)", "stroke-width": 1, opacity: 0.3,
  }));

  penEl.style.top = `${y(now)}px`;

  const full = x1 - x0 + 2;
  if (reduced || revealed) {
    wipe.setAttribute("width", full);
  } else {
    revealed = true;
    try {
      wipe.animate(
        [{ width: "0px" }, { width: `${full}px` }],
        { duration: 950, easing: "cubic-bezier(.25,.6,.3,1)", fill: "forwards" }
      );
    } catch {
      wipe.setAttribute("width", full);
    }
  }
}

/* -- readouts ---------------------------------------------- */

function countUp(node, to, decimals, ms = 900) {
  if (reduced) {
    node.textContent = to.toFixed(decimals);
    return;
  }
  const start = performance.now();
  const step = (t) => {
    const p = Math.min(1, (t - start) / ms);
    const eased = 1 - Math.pow(1 - p, 3);
    node.textContent = (to * eased).toFixed(decimals);
    if (p < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

function hms(total) {
  const h = Math.floor(total / 3600);
  const m = String(Math.floor((total % 3600) / 60)).padStart(2, "0");
  const s = String(total % 60).padStart(2, "0");
  return `${h}:${m}:${s}`;
}

function startClock() {
  const node = document.getElementById("reset-clock");
  let left = snapshot.resetsInSeconds;
  const tick = () => {
    node.textContent = hms(Math.max(0, left));
    left -= 1;
  };
  tick();
  setInterval(tick, 1000);
}

function fillFooter() {
  const bar = document.getElementById("segbar");
  const { pct, segments } = snapshot.weekly;
  const lit = Math.round((pct / 100) * segments);
  for (let i = 0; i < segments; i++) {
    const seg = document.createElement("i");
    if (i < lit) seg.className = pct >= 80 && i >= segments * 0.8 ? "hot" : "on";
    bar.append(seg);
  }
  document.getElementById("weekly-pct").textContent = pct;
}

function wirePens() {
  for (const btn of document.querySelectorAll(".pen-ch")) {
    btn.addEventListener("click", () => {
      const id = btn.dataset.ch;
      const off = hidden.has(id);
      off ? hidden.delete(id) : hidden.add(id);
      btn.setAttribute("aria-pressed", String(off));
      drawChart();
    });
  }
}

function wireWindowControls() {
  const api = globalThis.__TAURI__?.window;
  for (const btn of document.querySelectorAll(".ctl")) {
    btn.addEventListener("click", () => {
      const win = api?.getCurrentWindow?.();
      if (!win) return;
      btn.dataset.win === "close" ? win.close() : win.minimize();
    });
  }
}

/* -- boot --------------------------------------------------- */

function boot() {
  drawChart();
  countUp(document.getElementById("pct-value"), snapshot.used, 1);
  countUp(document.getElementById("burn-value"), snapshot.burnPerMin / 1000, 1);
  document.getElementById("full-value").textContent =
    `${Math.floor(snapshot.minutesToLimit / 60)}:${String(snapshot.minutesToLimit % 60).padStart(2, "0")}`;
  startClock();
  fillFooter();
  wirePens();
  wireWindowControls();
  addEventListener("resize", drawChart);
}

document.fonts?.ready.then(boot) ?? boot();
