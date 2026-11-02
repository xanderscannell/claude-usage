/* Synthetic block, in the exact shape the Rust `get_snapshot` command
   returns, so the panel can be worked on in a browser. */

const BUCKETS = 60;
const BLOCK_MS = 5 * 60 * 60 * 1000;

function mulberry32(seed) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function mockSnapshot() {
  const now = Date.now();
  const start = Math.floor((now - 2.4 * 60 * 60 * 1000) / 3_600_000) * 3_600_000;
  const filled = Math.min(BUCKETS, Math.ceil((now - start) / (BLOCK_MS / BUCKETS)));
  const rnd = mulberry32(0x5eed11);

  const shares = [
    { id: "claude-opus-5", label: "Opus 5", rate: 1.6 },
    { id: "claude-sonnet-5", label: "Sonnet 5", rate: 0.7 },
    { id: "claude-haiku-4-5", label: "Haiku 4.5", rate: 0.15 },
  ];

  const series = shares.map((s) => {
    const values = [];
    let acc = 0;
    for (let i = 0; i < filled; i++) {
      acc += s.rate * (0.35 + rnd() * 1.3);
      values.push(acc);
    }
    return { id: s.id, label: s.label, values };
  });

  const used = series.reduce((sum, s) => sum + (s.values.at(-1) ?? 0), 0);
  const burn = 17.4;
  const remainingMin = Math.max(0, Math.round((start + BLOCK_MS - now) / 60000));
  const perMin = burn / 60;
  const toLimit = Math.ceil((100 - used) / perMin);

  return {
    source: "estimated",
    liveError: "Offline · showing last known limit",
    limitTokens: 17_148_739,
    blockStartMs: start,
    blockEndMs: start + BLOCK_MS,
    nowMs: now,
    resetsInSeconds: Math.round((start + BLOCK_MS - now) / 1000),
    series,
    filledBuckets: filled,
    usedPct: used,
    burnPctPerHour: burn,
    minutesToLimit: toLimit <= remainingMin ? toLimit : null,
    projectedEndPct: used + perMin * remainingMin,
    models: series.map((s) => ({
      id: s.id,
      label: s.label,
      weighted: (s.values.at(-1) ?? 0) * 171_487,
      pct: s.values.at(-1) ?? 0,
    })),
    messages: 59,
    rawTokens: 6_750_000,
    weekly: {
      source: "estimated",
      pct: 61.8,
      limitTokens: 90_000_000,
      weighted: 55_620_000,
      resetsAtMs: null,
    },
  };
}

/* Synthetic `stats` event, in the shape the Rust sampler emits. */
let load = 30;
export function mockStats() {
  load = Math.max(3, Math.min(97, load + (Math.random() - 0.5) * 18));
  const G = 1024 ** 3;
  return {
    cpuUsage: load,
    cpuFreqMhz: 4200 + Math.round(Math.random() * 500),
    cpuBrand: "AMD Ryzen 7 7800X3D 8-Core Processor",
    cpuTemp: 48 + load / 5,
    memTotal: 32 * G,
    memUsed: 14.2 * G,
    swapTotal: 8 * G,
    swapUsed: 0.6 * G,
    gpu: {
      name: "NVIDIA GeForce RTX 4080",
      temp: 52,
      util: Math.round(load * 0.8),
      clockMhz: 2610,
      fan: 38,
      memUsed: 6.1 * G,
      memTotal: 16 * G,
      powerW: 120 + load * 2,
    },
  };
}
