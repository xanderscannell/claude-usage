/* Mock telemetry. Swap this module for Tauri IPC when the
   backend lands - everything downstream reads `snapshot`. */

const BUCKETS = 60; // 5 minutes each, five hours of paper

/* Hand-authored control points, in percent of the 5-hour limit.
   The story the paper tells: a near-miss with the ceiling two
   hours back, roll-off as that work aged out of the window,
   and a fresh climb underway now. */
const CHANNELS = [
  { id: "opus", wash: 0.5, label: "Opus 4.5", ink: "var(--ink-blue)", ui: "var(--ui-blue)",
    keys: [3, 5, 18, 41, 55, 44, 26, 30, 41.2] },
  { id: "sonnet", wash: 0.5, label: "Sonnet 4.6", ink: "var(--ink-green)", ui: "var(--ui-green)",
    keys: [2, 6, 11, 19, 28, 24, 15, 17, 21.9] },
  { id: "haiku", wash: 0.36, label: "Haiku 4.5", ink: "var(--ink-graphite)", ui: "var(--ui-graphite)",
    keys: [0.5, 1, 3, 6, 9, 7, 4, 4.5, 5.3] },
];

function mulberry32(seed) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Catmull-Rom through the control points, resampled to n values. */
function resample(keys, n) {
  const out = [];
  const last = keys.length - 1;
  const at = (i) => keys[Math.min(last, Math.max(0, i))];

  for (let s = 0; s < n; s++) {
    const u = (s / (n - 1)) * last;
    const i = Math.min(last - 1, Math.floor(u));
    const t = u - i;
    const [p0, p1, p2, p3] = [at(i - 1), at(i), at(i + 1), at(i + 2)];
    const v =
      0.5 *
      (2 * p1 +
        (-p0 + p2) * t +
        (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t +
        (-p0 + 3 * p1 - 3 * p2 + p3) * t * t * t);
    out.push(v);
  }
  return out;
}

function buildSeries() {
  const rnd = mulberry32(0x5eed11);
  return CHANNELS.map((ch) => {
    const smooth = resample(ch.keys, BUCKETS);
    const values = smooth.map((v, i) => {
      const jitter = i === BUCKETS - 1 ? 0 : (rnd() - 0.5) * 1.35;
      return Math.max(0, v + jitter * Math.min(1, v / 4));
    });
    return { ...ch, values, current: values[values.length - 1] };
  });
}

const series = buildSeries();
const used = series.reduce((sum, ch) => sum + ch.current, 0);
const totalTokens = 1_240_000;
const burnPerMin = 12_400;
const windowTokens = Math.round(totalTokens / (used / 100));
const minutesToLimit = Math.round((windowTokens - totalTokens) / burnPerMin);

export const snapshot = {
  bucketMinutes: 5,
  windowHours: 5,
  limitPct: 100,
  series,
  used,
  peak: Math.max(...series[0].values.map((_, i) =>
    series.reduce((s, ch) => s + ch.values[i], 0))),
  burnPerMin,
  minutesToLimit,
  totalTokens,
  messages: 412,
  resetsInSeconds: 2 * 3600 + 14 * 60 + 7,
  weekly: { pct: 58, segments: 12, resetsLabel: "Resets Mon 00:00" },
};
