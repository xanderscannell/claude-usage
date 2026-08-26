/* The paper. One 5-hour block end to end: the pen sits at now, and the
   blank paper to its right is the time still left in the block. */

const NS = "http://www.w3.org/2000/svg";
const BUCKETS = 60;
const BLOCK_MS = 5 * 60 * 60 * 1000;
const BUCKET_MS = BLOCK_MS / BUCKETS;

/* 100% sits at 100/112 of the plot height, leaving room for the
   over-range band above the limit line. */
const Y_MAX = 112;
const PAD = { left: 34, right: 26, top: 8, bottom: 19 };

/* Oxide red is reserved for the scale and the ceiling, never a model. */
export const INKS = ["var(--ink-blue)", "var(--ink-green)", "var(--ink-graphite)", "var(--ink-violet)"];
export const UI_INKS = ["var(--ui-blue)", "var(--ui-green)", "var(--ui-graphite)", "var(--ui-violet)"];
const WASH = [0.5, 0.5, 0.36, 0.44];

/* Presentation attributes cannot resolve var(), so paint goes to style. */
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

/* Blocks always open on the hour, so the axis needs no minutes; the
   limit crossing does. */
const clockLabel = (ms, withMinutes = false) =>
  new Date(ms)
    .toLocaleTimeString([], withMinutes ? { hour: "numeric", minute: "2-digit" } : { hour: "numeric" })
    .replace(/\s/g, "");

let revealed = false;

export function drawChart({ svg, paper, penEl, snap, hidden, reduced }) {
  const { width, height } = paper.getBoundingClientRect();
  const W = Math.round(width);
  const H = Math.round(height);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.replaceChildren();

  const x0 = PAD.left, x1 = W - PAD.right, y0 = PAD.top, y1 = H - PAD.bottom;
  const x = (i) => x0 + (i / BUCKETS) * (x1 - x0);
  const y = (v) => y1 - (Math.min(v, Y_MAX) / Y_MAX) * (y1 - y0);

  const filled = snap.filledBuckets ?? 0;
  /* The newest bucket is still filling, so its boundary sits ahead of
     now. Putting the pen at the true elapsed position keeps the x axis
     and the time axis in step - the projected crossing time is read off
     that correspondence, so a boundary here skewed it by up to a bucket. */
  const elapsed = snap.blockStartMs
    ? Math.min(BUCKETS, Math.max(0, (snap.nowMs - snap.blockStartMs) / BUCKET_MS))
    : filled;
  const penX = x(elapsed);
  const xAt = (i) => Math.min(x(i + 1), penX);

  /* --- printed scale ------------------------------------------- */
  const defs = el("defs");
  const hatch = el("pattern", {
    id: "hatch", width: 6, height: 6,
    patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)",
  });
  hatch.append(el("line", { x1: 0, y1: 0, x2: 0, y2: 6, stroke: "var(--grid)", "stroke-width": 1, opacity: 0.22 }));
  const clip = el("clipPath", { id: "feed" });
  const wipe = el("rect", { x: x0 - 1, y: 0, width: 0, height: H });
  clip.append(wipe);
  defs.append(hatch, clip);
  svg.append(defs);

  const printed = el("g", { "shape-rendering": "crispEdges" });
  for (let k = 0; k <= 20; k++) {
    const gx = x((BUCKETS * k) / 20);
    printed.append(el("line", {
      x1: gx, y1: y0, x2: gx, y2: y1,
      stroke: "var(--grid)", "stroke-width": 1, opacity: k % 4 === 0 ? 0.26 : 0.11,
    }));
  }
  for (let v = 0; v <= 110; v += 10) {
    printed.append(el("line", {
      x1: x0, y1: y(v), x2: x1, y2: y(v),
      stroke: "var(--grid)", "stroke-width": 1, opacity: v % 25 === 0 ? 0.26 : 0.11,
    }));
  }
  svg.append(printed);

  /* --- over-range band ----------------------------------------- */
  const band = el("g");
  band.append(el("rect", { x: x0, y: y0, width: x1 - x0, height: y(100) - y0, fill: "url(#hatch)" }));
  band.append(el("line", { x1: x0, y1: y(100), x2: x1, y2: y(100), stroke: "var(--grid)", "stroke-width": 1.3, opacity: 0.85 }));
  band.append(el("text", {
    x: x0 + 5, y: y(100) - 5, fill: "var(--grid)", opacity: 0.8,
    "font-family": "var(--font-silk)", "font-size": 7.5, "font-weight": 600, "letter-spacing": "0.16em",
  }, "LIMIT"));
  svg.append(band);

  /* --- scale numerals and the block's clock -------------------- */
  const marks = el("g", {
    fill: "var(--grid)", opacity: 0.72,
    "font-family": "var(--font-silk)", "font-size": 7.5, "font-weight": 500,
  });
  for (const v of [0, 25, 50, 75, 100]) {
    marks.append(el("text", { x: x0 - 7, y: y(v) + 2.6, "text-anchor": "end" }, String(v)));
  }
  if (snap.blockStartMs) {
    // Hour-only labels are only honest when the block starts on the
    // hour; otherwise every mark would read an hour early.
    const onHour = new Date(snap.blockStartMs).getMinutes() === 0;
    for (let h = 0; h <= 5; h++) {
      marks.append(el("text", {
        x: x(h * 12), y: y1 + 12,
        "text-anchor": h === 0 ? "start" : h === 5 ? "end" : "middle",
      }, clockLabel(snap.blockStartMs + h * 3_600_000, !onHour)));
    }
  }
  svg.append(marks);

  /* --- the pens ------------------------------------------------- */
  const ink = el("g", { "clip-path": "url(#feed)" });
  const base = new Array(filled).fill(0);
  const order = snap.series.map((s, i) => ({ ...s, ink: INKS[i % INKS.length], wash: WASH[i % WASH.length] }));

  /* Graphite sits at the baseline so it never fuses with the
     graphite total pen riding the top of the stack. */
  for (const ch of [...order].reverse()) {
    const top = base.map((b, i) => b + (ch.values[i] ?? 0));
    if (!hidden.has(ch.id) && filled > 0) {
      const up = top.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`);
      const down = base.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`).reverse();
      ink.append(el("polygon", {
        points: [`${x(0).toFixed(1)},${y(0)}`, ...up, ...down].join(" "),
        fill: ch.ink, opacity: ch.wash,
      }));
      ink.append(el("polyline", {
        points: [`${x(0).toFixed(1)},${y(0)}`, ...up].join(" "),
        fill: "none", stroke: ch.ink, "stroke-width": 1.1, "stroke-linejoin": "round", opacity: 0.85,
      }));
    }
    for (let i = 0; i < filled; i++) base[i] = top[i];
  }

  const used = snap.usedPct ?? 0;

  /* the total is its own pen, and it always writes */
  if (filled > 0) {
    ink.append(el("polyline", {
      points: [`${x(0).toFixed(1)},${y(0)}`, ...base.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`)].join(" "),
      fill: "none", stroke: "var(--ink-graphite)",
      "stroke-width": 1.7, "stroke-linejoin": "round", "stroke-linecap": "round",
    }));
  }

  /* --- where this block is heading ------------------------------ */
  const projected = snap.projectedEndPct ?? used;
  const endX = x(BUCKETS);
  const dash = { fill: "none", "stroke-dasharray": "3 3" };

  if (snap.blockEndMs && penX < endX - 0.5 && projected > used + 0.05) {
    if (projected > 100 && used < 100) {
      /* Usage cannot actually climb past the ceiling - you are held at
         it - so the projection rises to 100% and then runs along it.
         The red stretch is how long this block would spend rate
         limited, which is why it ends at the reset and not before. */
      const t = (100 - used) / (projected - used);
      const cx = penX + t * (endX - penX);
      const ct = snap.nowMs + t * (snap.blockEndMs - snap.nowMs);
      const nearRight = cx > (x0 + endX) / 2;

      ink.append(el("polyline", {
        points: `${penX.toFixed(1)},${y(used).toFixed(1)} ${cx.toFixed(1)},${y(100).toFixed(1)}`,
        stroke: "var(--ink-graphite)", "stroke-width": 1.2, opacity: 0.55, ...dash,
      }));
      ink.append(el("polyline", {
        points: `${cx.toFixed(1)},${y(100).toFixed(1)} ${endX.toFixed(1)},${y(100).toFixed(1)}`,
        stroke: "var(--grid)", "stroke-width": 1.9, opacity: 0.9, ...dash,
      }));
      ink.append(el("line", {
        x1: cx, y1: y(100) - 5, x2: cx, y2: y(100) + 5,
        stroke: "var(--grid)", "stroke-width": 1.4,
      }));
      ink.append(el("text", {
        x: nearRight ? cx - 5 : cx + 5, y: y(100) - 8,
        "text-anchor": nearRight ? "end" : "start",
        fill: "var(--grid)", "font-family": "var(--font-silk)",
        "font-size": 7, "font-weight": 700, "letter-spacing": "0.13em",
      }, `100% AT ${clockLabel(ct, true)}`));
    } else if (used >= 100) {
      ink.append(el("polyline", {
        points: `${penX.toFixed(1)},${y(used).toFixed(1)} ${endX.toFixed(1)},${y(used).toFixed(1)}`,
        stroke: "var(--grid)", "stroke-width": 1.9, opacity: 0.9, ...dash,
      }));
    } else {
      ink.append(el("polyline", {
        points: `${penX.toFixed(1)},${y(used).toFixed(1)} ${endX.toFixed(1)},${y(projected).toFixed(1)}`,
        stroke: "var(--ink-graphite)", "stroke-width": 1.2, opacity: 0.5, ...dash,
      }));
      ink.append(el("text", {
        x: endX - 3, y: y(projected) - 7, "text-anchor": "end",
        fill: "var(--ink-graphite)", opacity: 0.72,
        "font-family": "var(--font-silk)", "font-size": 7, "font-weight": 600, "letter-spacing": "0.13em",
      }, `TRENDING ${projected.toFixed(0)}`));
    }
  }
  svg.append(ink);

  /* the pen's rail at now */
  svg.append(el("line", { x1: penX, y1: y0, x2: penX, y2: y1, stroke: "var(--ink-graphite)", "stroke-width": 1, opacity: 0.34 }));

  penEl.style.top = `${y(used)}px`;
  penEl.style.left = `${penX}px`;

  const full = x1 - x0 + 2;
  if (reduced || revealed) {
    wipe.setAttribute("width", full);
  } else {
    revealed = true;
    try {
      wipe.animate([{ width: "0px" }, { width: `${full}px` }],
        { duration: 950, easing: "cubic-bezier(.25,.6,.3,1)", fill: "forwards" });
    } catch {
      wipe.setAttribute("width", full);
    }
  }
}
