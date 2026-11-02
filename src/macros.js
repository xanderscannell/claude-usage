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
