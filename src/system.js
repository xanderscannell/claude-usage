/* The System page: CPU, memory and GPU, pushed by the Rust sampler as
   `stats` events. Falls back to synthetic stats in a plain browser. */

import { mockStats } from "./mock.js";

const HISTORY = 64;
// Ink on paper, matching the usage chart's pens.
const INK = { cpu: "#1F4E9C", gpu: "#5B3E8E" };
const GRID = "rgba(194, 81, 58, 0.28)";

const $ = (id) => document.getElementById(id);
const cpuData = new Array(HISTORY).fill(0);
const gpuData = new Array(HISTORY).fill(0);
let stale;

const gib = (bytes) => bytes / 1024 ** 3;
const memPair = (used, total) => `${gib(used).toFixed(1)} / ${gib(total).toFixed(1)} G`;
const pct = (used, total) => (total > 0 ? (used / total) * 100 : 0);

function bar(id, value) {
  $(id).style.setProperty("--fill", `${Math.min(100, value)}%`);
}

function fitCanvas(c) {
  const rect = c.getBoundingClientRect();
  const dpr = devicePixelRatio || 1;
  c.width = Math.max(1, Math.round(rect.width * dpr));
  c.height = Math.max(1, Math.round(rect.height * dpr));
}

function drawSparkline(canvas, data, color) {
  fitCanvas(canvas);
  const ctx = canvas.getContext("2d");
  const W = canvas.width;
  const H = canvas.height;
  const dpr = devicePixelRatio || 1;
  ctx.clearRect(0, 0, W, H);

  const stepX = W / (data.length - 1);
  const yOf = (v) => H - (Math.max(0, Math.min(100, v)) / 100) * (H - 4 * dpr) - 2 * dpr;

  // Quarter rules, in the chart paper's oxide red.
  ctx.strokeStyle = GRID;
  ctx.lineWidth = dpr;
  for (const q of [25, 50, 75]) {
    const y = Math.round(yOf(q)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(W, y);
    ctx.stroke();
  }

  ctx.beginPath();
  ctx.moveTo(0, H);
  data.forEach((v, i) => ctx.lineTo(i * stepX, yOf(v)));
  ctx.lineTo(W, H);
  ctx.closePath();
  ctx.fillStyle = color + "40";
  ctx.fill();

  ctx.beginPath();
  data.forEach((v, i) => (i === 0 ? ctx.moveTo(0, yOf(v)) : ctx.lineTo(i * stepX, yOf(v))));
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.6 * dpr;
  ctx.lineJoin = "round";
  ctx.stroke();
}

function render(s) {
  cpuData.push(s.cpuUsage);
  cpuData.shift();
  $("cpu-pct").textContent = Math.round(s.cpuUsage);
  $("cpu-clock").textContent = `${(s.cpuFreqMhz / 1000).toFixed(2)} GHz`;
  // Windows only exposes CPU temperature to admin tools, so hide it rather than show a blank.
  $("cpu-temp").hidden = s.cpuTemp == null;
  if (s.cpuTemp != null) $("cpu-temp").textContent = `${Math.round(s.cpuTemp)}°`;
  if (s.cpuBrand) $("cpu-name").textContent = s.cpuBrand;
  $("cpu-ram").textContent = memPair(s.memUsed, s.memTotal);
  bar("bar-ram", pct(s.memUsed, s.memTotal));
  $("cpu-swap").textContent = s.swapTotal > 0 ? memPair(s.swapUsed, s.swapTotal) : "—";
  bar("bar-swap", pct(s.swapUsed, s.swapTotal));
  drawSparkline($("cpu-canvas"), cpuData, INK.cpu);

  const g = s.gpu;
  $("gpu-row").hidden = !g;
  if (g) {
    gpuData.push(g.util);
    gpuData.shift();
    if (g.name) $("gpu-name").textContent = g.name;
    $("gpu-pct").textContent = g.util;
    $("gpu-clock").textContent = `${g.clockMhz} MHz · ${g.powerW.toFixed(0)} W`;
    $("gpu-temp").textContent = `${g.temp}°`;
    $("gpu-vram").textContent = memPair(g.memUsed, g.memTotal);
    bar("bar-vram", pct(g.memUsed, g.memTotal));
    $("gpu-fan").textContent = `${g.fan}%`;
    bar("bar-fan", g.fan);
    drawSparkline($("gpu-canvas"), gpuData, INK.gpu);
  }

  // The lamp means samples are arriving; it goes dark if they stop.
  const rec = $("sys-rec");
  rec.classList.remove("rec--dark");
  clearTimeout(stale);
  stale = setTimeout(() => rec.classList.add("rec--dark"), 5000);
}

function paintClock() {
  const now = new Date();
  $("sys-clock").textContent = now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  $("sys-date").textContent = now.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
}

export function wireSystem() {
  paintClock();
  setInterval(paintClock, 1000);
  const events = globalThis.__TAURI__?.event;
  if (events) events.listen("stats", (e) => render(e.payload));
  else setInterval(() => render(mockStats()), 1500);
}
