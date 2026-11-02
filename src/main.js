import { alignWindow, getSnapshot, live } from "./api.js";
import { drawChart, UI_INKS } from "./chart.js";

const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
const POLL_MS = 5000;

const $ = (id) => document.getElementById(id);
const svg = $("chart");
const paper = svg.parentElement;
const penEl = $("pen");

const hidden = new Set();
let snap = null;
let resetLeft = 0;
let penKey = "";

/* -- readouts ----------------------------------------------------- */

function ease(node, to, decimals) {
  const from = Number.parseFloat(node.textContent) || 0;
  if (reduced || Math.abs(to - from) < 0.05) {
    node.textContent = to.toFixed(decimals);
    return;
  }
  const start = performance.now();
  const step = (t) => {
    const p = Math.min(1, (t - start) / 700);
    const k = 1 - Math.pow(1 - p, 3);
    node.textContent = (from + (to - from) * k).toFixed(decimals);
    if (p < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

const hms = (s) =>
  `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;

function paintClock() {
  $("reset-clock").textContent = snap?.blockStartMs ? hms(Math.max(0, resetLeft)) : "—";
}

const dayLabel = (ms) =>
  new Date(ms).toLocaleString([], { weekday: "short", hour: "numeric" }).replace(/,/, "");

function compact(n) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return String(Math.round(n));
}

/* -- pen legend --------------------------------------------------- */

function paintPens() {
  const host = $("pens");
  const key = snap.series.map((s) => s.id).join("|");

  if (key !== penKey) {
    penKey = key;
    host.replaceChildren();
    snap.series.forEach((s, i) => {
      const nib = UI_INKS[i % UI_INKS.length];
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "pen-ch";
      btn.dataset.ch = s.id;
      btn.setAttribute("aria-pressed", String(!hidden.has(s.id)));
      // Built with DOM calls rather than innerHTML: nothing here is
      // attacker-controlled today, but a label read off disk is one
      // careless edit away from being interpolated into markup.
      const part = (cls, tag = "span") => {
        const n = document.createElement(tag);
        n.className = cls;
        return n;
      };
      const swatch = part("pen-ch__nib");
      swatch.style.setProperty("--nib", nib);
      const stat = part("silk pen-ch__stat");
      stat.append(document.createElement("b"), "%");
      const bar = part("pen-ch__bar");
      const fill = document.createElement("i");
      fill.style.setProperty("--nib", nib);
      bar.append(fill);
      btn.append(swatch, part("pen-ch__name"), stat, bar);
      btn.addEventListener("click", () => {
        const off = hidden.has(s.id);
        off ? hidden.delete(s.id) : hidden.add(s.id);
        btn.setAttribute("aria-pressed", String(off));
        render();
      });
      host.append(btn);
    });
  }

  const top = Math.max(1, ...snap.models.map((m) => m.pct));
  snap.series.forEach((s, i) => {
    const btn = host.children[i];
    if (!btn) return;
    const pct = snap.models.find((m) => m.id === s.id)?.pct ?? 0;
    btn.querySelector(".pen-ch__name").textContent = s.label;
    btn.querySelector(".pen-ch__stat b").textContent = pct.toFixed(1);
    btn.querySelector(".pen-ch__bar i").style.setProperty("--fill", `${(pct / top) * 100}%`);
  });
}

/* -- footer ------------------------------------------------------- */

function paintFooter() {
  const bar = $("segbar");
  const segments = 12;
  const pct = snap.weekly.pct;
  const lit = Math.min(segments, Math.round((pct / 100) * segments));
  if (bar.children.length !== segments) {
    bar.replaceChildren(...Array.from({ length: segments }, () => document.createElement("i")));
  }
  [...bar.children].forEach((seg, i) => {
    seg.className = i < lit ? (pct >= 80 && i >= segments * 0.8 ? "hot" : "on") : "";
  });
  $("weekly-pct").textContent = pct.toFixed(0);

  // The endpoint names the real weekly reset; local logs can only guess,
  // so without it we say nothing rather than invent a schedule.
  const wr = snap.weekly.resetsAtMs;
  $("weekly-label").textContent = wr ? "Weekly" : "7-day";
  $("weekly-reset").textContent = wr ? `resets ${dayLabel(wr)}` : "";

  const state = $("source-state");
  state.textContent = { live: "Live", remembered: "Remembered", estimated: "Estimated" }[snap.source];
  state.classList.toggle("silk--warn", snap.source === "estimated");
  state.classList.toggle("silk--live", snap.source === "live");

  // With the legend hidden, the footer is the only place the model is named.
  const solo = snap.series.length <= 1 ? snap.series[0]?.label : null;
  $("foot-right").textContent = [
    solo,
    `${compact(snap.rawTokens)} tokens`,
    `${snap.messages} message${snap.messages === 1 ? "" : "s"}`,
  ].filter(Boolean).join(" · ");
}

/* -- render ------------------------------------------------------- */

function render() {
  if (!snap) return;
  const open = Boolean(snap.blockStartMs);
  const solo = snap.series.length <= 1;
  document.querySelector(".case").classList.toggle("case--solo", solo);

  $("paper-empty").hidden = open;
  penEl.hidden = !open;
  ease($("pct-value"), snap.usedPct, 1);
  ease($("burn-value"), snap.burnPctPerHour, 1);

  const mins = snap.minutesToLimit;
  $("full-value").textContent =
    mins != null
      ? `${Math.floor(mins / 60)}:${String(mins % 60).padStart(2, "0")}`
      : open && snap.projectedEndPct < 100
        ? "Clear"
        : "—";

  // With no sheet to open, the title rail is where the reason lives.
  $("unit-mark").textContent = snap.source === "live"
    ? `Live · limit ${compact(snap.limitTokens)}`
    : snap.liveError
      ?? (snap.source === "remembered"
        ? `Last known limit ${compact(snap.limitTokens)}`
        : "Estimated · scaled to your busiest block");
  // The lamp records live telemetry, so it goes dark when there is none.
  document.querySelector(".rec").classList.toggle("rec--dark", snap.source !== "live");

  svg.setAttribute(
    "aria-label",
    open
      ? `${snap.usedPct.toFixed(1)} percent of the 5-hour limit used, burning ${snap.burnPctPerHour.toFixed(1)} percent per hour.`
      : "No block open."
  );

  drawChart({ svg, paper, penEl, snap, hidden, reduced });
  paintPens();
  paintFooter();
  paintClock();
}

function wireWindowControls() {
  const api = globalThis.__TAURI__?.window;
  for (const btn of document.querySelectorAll(".ctl")) {
    btn.addEventListener("click", () => {
      if (btn.dataset.win === "align") return align(btn);
      const win = api?.getCurrentWindow?.();
      if (!win) return;
      btn.dataset.win === "close" ? win.close() : win.minimize();
    });
  }
  // The window is a fixed size, so it cannot be maximised; aligning it
  // to its display is the nearest thing that gesture can mean here.
  document.querySelector(".rail")?.addEventListener("dblclick", (e) => {
    if (!e.target.closest(".ctl")) align();
  });
}

/* Marks the button when the display is absent, so a request that could
   not be honoured says so rather than looking ignored. */
async function align(btn) {
  const moved = await alignWindow();
  if (moved || !btn) return;
  btn.classList.remove("is-miss");
  void btn.offsetWidth;
  btn.classList.add("is-miss");
}

/* -- pages --------------------------------------------------------- */

function wirePager() {
  const pager = $("pager");
  const pages = [...pager.children];
  const dots = $("dots");
  const at = () => Math.round(pager.scrollLeft / pager.clientWidth);
  const go = (i) => pager.scrollTo({
    left: Math.max(0, Math.min(pages.length - 1, i)) * pager.clientWidth,
    behavior: reduced ? "auto" : "smooth",
  });

  dots.replaceChildren(...pages.map((p, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "dot";
    b.setAttribute("aria-label", `${p.getAttribute("aria-label")} page`);
    b.addEventListener("click", () => go(i));
    return b;
  }));
  const mark = () => [...dots.children].forEach((d, i) => d.setAttribute("aria-current", String(i === at())));
  pager.addEventListener("scroll", mark, { passive: true });
  mark();

  addEventListener("keydown", (e) => {
    const step = { ArrowLeft: -1, ArrowRight: 1 }[e.key];
    if (!step) return;
    e.preventDefault();
    go(at() + step);
  });

  // Touch swipes scroll natively; a mouse cannot, so a drag stands in.
  // Buttons and the window's drag region keep their own meaning.
  let downX = null;
  pager.addEventListener("pointerdown", (e) => {
    if (e.pointerType === "mouse" && !e.target.closest("button, [data-tauri-drag-region]")) downX = e.clientX;
  });
  addEventListener("pointerup", (e) => {
    if (downX == null) return;
    const dx = e.clientX - downX;
    downX = null;
    if (Math.abs(dx) > 60) go(at() - Math.sign(dx));
  });
}

/* -- boot ---------------------------------------------------------- */

async function poll() {
  try {
    snap = await getSnapshot();
    resetLeft = snap.resetsInSeconds;
    render();
  } catch (e) {
    const node = $("paper-empty");
    node.hidden = false;
    node.textContent = String(e).replace(/^Error:\s*/, "");
  }
}

async function boot() {
  wireWindowControls();
  wirePager();
  if (!live) document.body.classList.add("is-demo");
  await poll();
  setInterval(poll, POLL_MS);
  setInterval(() => {
    if (resetLeft > 0) resetLeft -= 1;
    paintClock();
  }, 1000);
  addEventListener("resize", render);
}

document.fonts?.ready.then(boot) ?? boot();
