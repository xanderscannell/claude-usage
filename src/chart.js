/* The paper. One 5-hour block end to end: the pen sits at now, and the
   blank paper to its right is the time still left in the block. */

const NS = "http://www.w3.org/2000/svg";
const BUCKETS = 60;
const BLOCK_MS = 5 * 60 * 60 * 1000;
const BUCKET_MS = BLOCK_MS / BUCKETS;

/* 100% sits at 100/112 of the plot height, leaving room for the
   over-range band above the limit line. */
const Y_MAX = 112;
const PAD = { left: 44, right: 30, top: 10, bottom: 28 };

/* Red is reserved for the limit, never a model. */
export const INKS = ["var(--ink-blue)", "var(--ink-teal)", "var(--ink-amber)", "var(--ink-violet)"];
const TOTAL = "var(--text)";
const SCALE = "var(--faint)";

/* Presentation attributes cannot resolve var(), so paint goes to style. */
const PAINT = new Set([
  "fill", "stroke", "opacity", "stroke-width", "stroke-dasharray",
  "stroke-linejoin", "stroke-linecap", "shape-rendering",
  "font-family", "font-size", "font-weight", "letter-spacing", "filter", "stop-color",
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

/* Blocks are anchored at a message, not at the hour, so the axis
   usually needs minutes; the limit crossing always does. */
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
  hatch.append(el("line", { x1: 0, y1: 0, x2: 0, y2: 6, stroke: "var(--limit)", "stroke-width": 1, opacity: 0.16 }));
  const clip = el("clipPath", { id: "feed" });
  const wipe = el("rect", { x: x0 - 1, y: 0, width: 0, height: H });
  clip.append(wipe);
  defs.append(hatch, clip);
  /* each model's wash fades from its edge to the baseline, as on ponytail's board */
  INKS.forEach((ink, i) => {
    const g = el("linearGradient", { id: `wash${i}`, x1: 0, y1: 0, x2: 0, y2: 1 });
    g.append(
      el("stop", { offset: 0, "stop-color": ink, "stop-opacity": 0.55 }),
      el("stop", { offset: 1, "stop-color": ink, "stop-opacity": 0.12 }),
    );
    defs.append(g);
  });
  svg.append(defs);

  const printed = el("g", { "shape-rendering": "crispEdges" });
  for (let h = 0; h <= 5; h++) {
    printed.append(el("line", {
      x1: x(h * 12), y1: y0, x2: x(h * 12), y2: y1,
      stroke: "var(--rule)", "stroke-width": 1,
    }));
  }
  for (const v of [0, 25, 50, 75]) {
    printed.append(el("line", {
      x1: x0, y1: y(v), x2: x1, y2: y(v),
      stroke: "var(--rule)", "stroke-width": 1,
    }));
  }
  svg.append(printed);

  /* --- over-range band ----------------------------------------- */
  const band = el("g");
  band.append(el("rect", { x: x0, y: y0, width: x1 - x0, height: y(100) - y0, fill: "url(#hatch)" }));
  band.append(el("line", { x1: x0, y1: y(100), x2: x1, y2: y(100), stroke: "var(--limit)", "stroke-width": 1.5, opacity: 0.9 }));
  svg.append(band);

  /* --- scale numerals and the block's clock -------------------- */
  const marks = el("g", {
    fill: SCALE, "font-family": "var(--font-mono)", "font-size": 14, "font-weight": 500,
  });
  for (const v of [0, 25, 50, 75]) {
    marks.append(el("text", { x: x0 - 8, y: y(v) + 5, "text-anchor": "end" }, String(v)));
  }
  marks.append(el("text", { x: x0 - 8, y: y(100) + 5, "text-anchor": "end", fill: "var(--limit)" }, "100"));
  if (snap.blockStartMs) {
    // Hour-only labels are only honest on the rare block that does start
    // on the hour; otherwise every mark would read an hour early.
    const onHour = new Date(snap.blockStartMs).getMinutes() === 0;
    for (let h = 0; h <= 5; h++) {
      marks.append(el("text", {
        x: x(h * 12), y: y1 + 21,
        "text-anchor": h === 0 ? "start" : h === 5 ? "end" : "middle",
      }, clockLabel(snap.blockStartMs + h * 3_600_000, !onHour)));
    }
  }
  svg.append(marks);

  /* --- the pens ------------------------------------------------- */
  const ink = el("g", { "clip-path": "url(#feed)" });
  const base = new Array(filled).fill(0);
  const order = snap.series.map((s, i) => ({ ...s, ink: INKS[i % INKS.length], wash: `url(#wash${i % INKS.length})` }));

  for (const ch of [...order].reverse()) {
    const top = base.map((b, i) => b + (ch.values[i] ?? 0));
    if (!hidden.has(ch.id) && filled > 0) {
      const up = top.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`);
      const down = base.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`).reverse();
      ink.append(el("polygon", {
        points: [`${x(0).toFixed(1)},${y(0)}`, ...up, ...down].join(" "),
        fill: ch.wash,
      }));
      ink.append(el("polyline", {
        points: [`${x(0).toFixed(1)},${y(0)}`, ...up].join(" "),
        fill: "none", stroke: ch.ink, "stroke-width": 2, "stroke-linejoin": "round",
        filter: `drop-shadow(0 0 5px ${ch.ink})`,
      }));
    }
    for (let i = 0; i < filled; i++) base[i] = top[i];
  }

  const used = snap.usedPct ?? 0;

  /* the total is its own pen, and it always writes */
  if (filled > 0) {
    ink.append(el("polyline", {
      points: [`${x(0).toFixed(1)},${y(0)}`, ...base.map((v, i) => `${xAt(i).toFixed(1)},${y(v).toFixed(1)}`)].join(" "),
      fill: "none", stroke: TOTAL,
      "stroke-width": 2.5, "stroke-linejoin": "round", "stroke-linecap": "round",
    }));
  }

  /* --- where this block is heading ------------------------------ */
  const projected = snap.projectedEndPct ?? used;
  const endX = x(BUCKETS);
  const dash = { fill: "none", "stroke-dasharray": "5 5" };

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
        stroke: TOTAL, "stroke-width": 1.5, opacity: 0.5, ...dash,
      }));
      ink.append(el("polyline", {
        points: `${cx.toFixed(1)},${y(100).toFixed(1)} ${endX.toFixed(1)},${y(100).toFixed(1)}`,
        stroke: "var(--limit)", "stroke-width": 1.9, opacity: 0.9, ...dash,
      }));
      ink.append(el("line", {
        x1: cx, y1: y(100) - 5, x2: cx, y2: y(100) + 5,
        stroke: "var(--limit)", "stroke-width": 1.4,
      }));
      ink.append(el("text", {
        x: nearRight ? cx - 7 : cx + 7, y: y(100) - 10,
        "text-anchor": nearRight ? "end" : "start",
        fill: "var(--limit)", "font-family": "var(--font-sans)",
        "font-size": 15, "font-weight": 600,
      }, `Limit at ${clockLabel(ct, true)}`));
    } else if (used >= 100) {
      ink.append(el("polyline", {
        points: `${penX.toFixed(1)},${y(used).toFixed(1)} ${endX.toFixed(1)},${y(used).toFixed(1)}`,
        stroke: "var(--limit)", "stroke-width": 1.9, opacity: 0.9, ...dash,
      }));
    } else {
      ink.append(el("polyline", {
        points: `${penX.toFixed(1)},${y(used).toFixed(1)} ${endX.toFixed(1)},${y(projected).toFixed(1)}`,
        stroke: TOTAL, "stroke-width": 1.5, opacity: 0.5, ...dash,
      }));
      ink.append(el("text", {
        x: endX - 3, y: y(projected) - 10, "text-anchor": "end",
        fill: "var(--muted)", "font-family": "var(--font-sans)", "font-size": 15, "font-weight": 500,
      }, `On pace for ${projected.toFixed(0)}%`));
    }
  }
  svg.append(ink);

  /* the pen's rail at now */
  svg.append(el("line", { x1: penX, y1: y0, x2: penX, y2: y1, stroke: TOTAL, "stroke-width": 1, opacity: 0.18 }));

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
