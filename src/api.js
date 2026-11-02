/* Talks to the Rust side. Falls back to a synthetic block when the page
   is opened in a plain browser, so `npm run dev` still renders. */

import { mockMacros, mockSnapshot } from "./mock.js";

const core = globalThis.__TAURI__?.core;
export const live = typeof core?.invoke === "function";

async function call(cmd, args) {
  return core.invoke(cmd, args);
}

export async function getSnapshot() {
  if (!live) return mockSnapshot();
  return call("get_snapshot");
}

/* Sends the window back to the display it is sized for. Resolves false
   when that display is not connected, so nothing moved. */
export async function alignWindow() {
  if (!live) return false;
  return call("align_window");
}


/* The Macros page. The page names a button by position and label only;
   what it does lives in macros.json on the Rust side. */
export async function getMacros() {
  if (!live) return mockMacros();
  return call("get_macros");
}

export async function runMacro(index, label) {
  if (!live) return;
  return call("run_macro", { index, label });
}
