// A small local performance log, to diagnose stalls in the real app (WebKit) that don't show up
// in Chrome. It records page jumps, render jobs and main-thread stalls, and appends them every
// few seconds to perf.log in the app's data folder (capped in size). Nothing leaves the Mac.

import { invoke } from "@tauri-apps/api/core";

const STALL_MS = 120;
let lines: string[] = [];
let started = false;
const t0 = performance.now();

function stamp() {
  return `${new Date().toISOString().slice(11, 23)} +${Math.round(performance.now() - t0)}`;
}

export const perf = {
  mark(event: string) {
    lines.push(`${stamp()} ${event}`);
    if (lines.length > 500) lines = lines.slice(-500);
  },
  start() {
    if (started) return;
    started = true;
    let last = performance.now();
    const tick = () => {
      const now = performance.now();
      // A frame that took this long means the window didn't respond to clicks or scrolling.
      if (now - last > STALL_MS && document.visibilityState === "visible") perf.mark(`stall ${Math.round(now - last)}ms`);
      last = now;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    window.setInterval(() => {
      if (lines.length === 0) return;
      const batch = lines;
      lines = [];
      invoke("perf_log", { lines: batch }).catch(() => {});
    }, 3000);
  },
};
