#!/usr/bin/env node
// Measures the shell's own motion in the real frontend, running in a headless Edge/Chrome
// against the dev mock (npm run dev). No backend, no live sites. Companion to
// motion-bench.mjs, which covers the queue rows and progress.
//
//   node scripts/dev/shell-bench.mjs [--browser <exe>] [--out file.json]
//
// With the sample queue (6 rows) and with 500 more rows, it records:
//   navigation   Queue -> Library -> Queue: script/layout/style milliseconds, long tasks,
//                frame pacing, and which view-transition parts are animating two frames in
//                (so a crossfade of the whole window can be told from the page alone);
//   sidebar      closing and opening the sidebar over the Queue: the same measures;
// and once, with the sample data:
//   libraryFilter  typing in the Library search: how many cards are moving, and pacing;
//   lookup         a link lookup: the box of what is shown while reading and of the
//                  details that replace it, so a jump between the two can be seen.
// Runs twice: with motion, and with prefers-reduced-motion: reduce emulated over CDP.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
const browser = args.browser ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const repo = resolve(import.meta.dirname, "..", "..");
const port = 1434;
const debugPort = 9336;

function waitForOutput(child, pattern) {
  return new Promise((resolvePromise, reject) => {
    let seen = "";
    const onData = (d) => {
      seen += d;
      if (pattern.test(seen)) resolvePromise();
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
    child.once("exit", (code) => reject(new Error(`exited ${code}: ${seen.slice(-500)}`)));
  });
}

const vite = spawn(process.execPath, [join(repo, "node_modules/vite/bin/vite.js"), "--port", String(port), "--strictPort"], { cwd: repo, stdio: ["ignore", "pipe", "pipe"] });
const profile = mkdtempSync(join(tmpdir(), "claws-shell-bench-"));
const chrome = spawn(browser, ["--headless=new", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`, "--no-first-run", "--window-size=1160,760", "about:blank"], { stdio: "ignore" });

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("no browser page");
}

let nextId = 1;
const pending = new Map();
const pageErrors = [];
let socket;
function cdp(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise((resolvePromise, reject) => pending.set(id, { resolvePromise, reject }));
}
async function evaluate(expression) {
  // A step that never answers (a frame that never comes) fails by name instead of hanging.
  const timeout = new Promise((_, reject) => setTimeout(() => reject(new Error(`timed out: ${expression.slice(0, 160)}`)), 30000));
  const result = await Promise.race([cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }), timeout]);
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 800));
  return result.result.value;
}
async function metrics() {
  const { metrics: list } = await cdp("Performance.getMetrics");
  const pick = (name) => list.find((m) => m.name === name)?.value ?? 0;
  return { script: pick("ScriptDuration") * 1000, layout: pick("LayoutDuration") * 1000, style: pick("RecalcStyleDuration") * 1000 };
}
const diff = (a, b) => ({ scriptMs: +(b.script - a.script).toFixed(1), layoutMs: +(b.layout - a.layout).toFixed(1), styleMs: +(b.style - a.style).toFixed(1) });
const takeLongTasks = () => evaluate("(() => { const d = window.__lt.splice(0); return { count: d.length, longestMs: d.length ? Math.round(Math.max(...d)) : 0 }; })()");

const pageHelpers = `
  window.__s = {
    emit: (event, payload) => window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event, payload }),
    frame: () => new Promise((r) => requestAnimationFrame(() => r())),
    wait: (ms) => new Promise((r) => setTimeout(r, ms)),
    revision: 9000,
    add(n) {
      const jobs = Array.from({ length: n }, (_, k) => ({ id: 's' + k, url: 'https://example.com/s/' + k, title: 'Shell job ' + k, thumbnail: null, uploader: 'Bench', duration: 60,
        options: { mode: 'video', maxHeight: null, videoPreset: 'compatible', audioFormat: 'mp3', subtitleLangs: [], autoSubs: false, embedSubs: false, fastSave: false, transcriptFormat: 'txt' },
        status: 'queued', progress: 0, progressKnown: false, speed: null, eta: null, filePath: null, error: null, createdAt: 1e13 + k,
        downloadedBytes: null, totalBytes: null, progressEstimated: false, revision: ++this.revision }));
      return this.emit('jobs-added', { schemaVersion: 1, jobs: jobs.map((j) => ({ schemaVersion: 1, jobId: j.id, revision: j.revision, job: j })) });
    },
    go(label) { [...document.querySelectorAll('.nav-item')].find((b) => b.querySelector('.nav-label')?.textContent === label).click(); },
    moving() {
      const names = {};
      for (const a of document.getAnimations()) {
        if (a.playState !== 'running') continue;
        const pseudo = a.effect?.pseudoElement;
        if (pseudo && pseudo.startsWith('::view-transition')) names[pseudo] = (names[pseudo] ?? 0) + 1;
      }
      return names;
    },
    tilesMoving() {
      return document.getAnimations().filter((a) => a.playState === 'running' && !(a instanceof CSSTransition) && !(a instanceof CSSAnimation) && a.effect?.target?.matches?.('.tile')).length;
    },
    box(selector) {
      const el = document.querySelector(selector);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { top: Math.round(r.top), height: Math.round(r.height), width: Math.round(r.width) };
    },
    async sample(ms, probe) {
      const gaps = [], samples = [];
      const start = performance.now();
      let last = start;
      while (performance.now() - start < ms) {
        await this.frame();
        const now = performance.now();
        gaps.push(now - last);
        last = now;
        if (probe) samples.push(probe());
      }
      return { frames: gaps.length, maxFrameMs: Math.round(Math.max(...gaps)), slowFrames: gaps.filter((g) => g > 25).length, samples };
    },
  };
  window.__lt = [];
  new PerformanceObserver((list) => list.getEntries().forEach((e) => window.__lt.push(e.duration))).observe({ type: 'longtask' });
`;

async function measure(trigger, ms, probe = "null") {
  await takeLongTasks();
  const before = await metrics();
  const pacing = await evaluate(`(async () => { ${trigger}; return window.__s.sample(${ms}, ${probe}); })()`);
  const after = await metrics();
  const long = await takeLongTasks();
  const { samples, ...frames } = pacing;
  return { ...diff(before, after), longTasks: long.count, longestTaskMs: long.longestMs, ...frames, samples };
}

async function run(reducedMotion) {
  await cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: reducedMotion ? "reduce" : "no-preference" }] });
  await cdp("Page.navigate", { url: `http://localhost:${port}/` });
  await new Promise((r) => setTimeout(r, 2500));
  await evaluate(pageHelpers);
  await evaluate("window.__clawsMockProbeScale = 0.4");
  const out = { reducedMotion, sizes: [] };

  for (const extra of [0, 500]) {
    if (extra > 0) {
      await evaluate(`window.__s.add(${extra})`);
      await evaluate("window.__s.wait(1500)");
    }
    await evaluate("window.__s.go('Queue')");
    await evaluate("window.__s.wait(900)");
    const rows = await evaluate("document.querySelectorAll('.jobs .job').length");
    const partsOf = (m) => {
      const seen = {};
      for (const s of m.samples.slice(1, 4)) for (const [name, n] of Object.entries(s ?? {})) seen[name] = Math.max(seen[name] ?? 0, n);
      const { samples, ...rest } = m;
      return { ...rest, transitionPartsMoving: seen };
    };
    const toLibrary = partsOf(await measure("window.__s.go('Library')", 700, "() => window.__s.moving()"));
    const toQueue = partsOf(await measure("window.__s.go('Queue')", 900, "() => window.__s.moving()"));
    await evaluate("window.__s.wait(600)");
    const strip = ({ samples, ...rest }) => rest;
    const close = strip(await measure("document.querySelector('.chrome .btn-glass').click()", 800));
    const open = strip(await measure("document.querySelector('.chrome .btn-glass').click()", 800));
    out.sizes.push({ queueRows: rows, navigation: { toLibrary, toQueue }, sidebar: { close, open } });
  }

  await evaluate("window.__s.go('Library')");
  await evaluate("window.__s.wait(900)");
  const typed = await measure(
    "const i = document.querySelector('.search-input'); i.value = 'ti'; i.dispatchEvent(new Event('input', { bubbles: true }))",
    700,
    "() => window.__s.tilesMoving()",
  );
  out.libraryFilter = {
    tiles: await evaluate("document.querySelectorAll('.tile').length"),
    mostTilesMoving: Math.max(0, ...typed.samples),
    scriptMs: typed.scriptMs, layoutMs: typed.layoutMs, styleMs: typed.styleMs, longTasks: typed.longTasks, frames: typed.frames, maxFrameMs: typed.maxFrameMs, slowFrames: typed.slowFrames,
  };

  await evaluate("window.__s.go('Download')");
  await evaluate("window.__s.wait(700)");
  out.lookup = await evaluate(`(async () => {
    const s = window.__s;
    const input = document.querySelector('.url-input');
    input.value = 'https://example.com/watch?v=shellbench';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    document.querySelector('.urlbar').requestSubmit();
    let reading = null, kind = null;
    for (let i = 0; i < 40 && !reading; i++) {
      await s.frame();
      for (const selector of ['.preview-skeleton', '.loading']) { if (!reading && document.querySelector(selector)) { reading = s.box(selector); kind = selector; } }
    }
    for (let i = 0; i < 400 && !document.querySelector('.preview:not(.preview-skeleton)'); i++) await s.frame();
    await s.wait(700);
    const details = s.box('.preview:not(.preview-skeleton)');
    return { whileReading: kind, readingBox: reading, detailsBox: details,
      topShiftPx: reading && details ? details.top - reading.top : null,
      heightChangePx: reading && details ? details.height - reading.height : null };
  })()`);
  out.pageErrors = [...pageErrors];
  return out;
}

let exitCode = 0;
try {
  await waitForOutput(vite, /ready in/i);
  socket = new WebSocket(await target());
  await new Promise((r) => socket.addEventListener("open", r, { once: true }));
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolvePromise, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolvePromise(message.result);
    } else if (message.method === "Runtime.exceptionThrown") {
      pageErrors.push(String(message.params?.exceptionDetails?.exception?.description ?? message.params?.exceptionDetails?.text).slice(0, 300));
    }
  });
  await cdp("Runtime.enable");
  await cdp("Page.enable");
  await cdp("Performance.enable");
  const results = [await run(false), await run(true)];
  for (const r of results) console.log(JSON.stringify(r));
  if (args.out) {
    writeFileSync(args.out, JSON.stringify(results, null, 2) + "\n");
    console.log(`wrote ${args.out}`);
  }
} catch (error) {
  console.error(error);
  exitCode = 1;
} finally {
  chrome.kill();
  vite.kill();
}
process.exit(exitCode);
