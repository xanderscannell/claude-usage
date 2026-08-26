import { getSnapshot, calibrate, resetCalibration, live } from "./api.js";
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

  const state = $("calibrate-state");
  state.textContent = { live: "Live", calibrated: "Calibrated", estimated: "Calibrate" }[snap.source];
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

  $("unit-mark").textContent = {
    live: `Live · limit ${compact(snap.limitTokens)}`,
    calibrated: `Calibrated · limit ${compact(snap.limitTokens)}`,
    estimated: "Estimated · scaled to your busiest block",
  }[snap.source];
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

/* -- calibration sheet -------------------------------------------- */

function openSheet() {
  $("sheet-lede").textContent = snap?.source === "live"
    ? "Claude Code's usage endpoint is answering, so the limit is already exact and nothing here is needed. You can still pin it by hand if you want to override."
    : "Nothing on this machine records your plan's ceiling. Run /usage in Claude Code and enter what it reports — your tokens are already counted exactly, this just sets what 100% means.";
  const err = snap?.liveError;
  if (err && snap?.source !== "live") {
    $("sheet-error").textContent = err;
    $("sheet-error").hidden = false;
  } else {
    $("sheet-error").hidden = true;
  }
  $("session-pct").value = "";
  $("weekly-pct").value = "";
  $("calibrate-clear").hidden = snap?.source !== "calibrated";
  $("sheet").hidden = false;
  $("session-pct").focus();
}

function closeSheet() {
  $("sheet").hidden = true;
  $("calibrate-open").focus();
}

function fail(message) {
  const node = $("sheet-error");
  node.textContent = String(message).replace(/^Error:\s*/, "");
  node.hidden = false;
}

function wireSheet() {
  $("calibrate-open").addEventListener("click", openSheet);
  $("calibrate-cancel").addEventListener("click", closeSheet);

  $("calibrate-clear").addEventListener("click", async () => {
    try {
      snap = await resetCalibration();
      render();
      closeSheet();
    } catch (e) {
      fail(e);
    }
  });

  $("calibrate-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const num = (id) => {
      const raw = $(id).value.trim();
      return raw === "" ? null : Number(raw);
    };
    const sessionPct = num("session-pct");
    const weeklyPct = num("weekly-pct");
    if (sessionPct == null && weeklyPct == null) {
      fail("Enter at least one percentage from /usage.");
      return;
    }
    try {
      snap = await calibrate({ sessionPct, weeklyPct });
      resetLeft = snap.resetsInSeconds;
      render();
      closeSheet();
    } catch (err) {
      fail(err);
    }
  });

  addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !$("sheet").hidden) closeSheet();
  });
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
  wireSheet();
  wireWindowControls();
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
