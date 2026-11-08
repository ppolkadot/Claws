#!/usr/bin/env node
// Measures a pasted batch of links in the real frontend, running in a headless Edge/Chrome
// against the dev mock (npm run dev). No backend, no live sites. Companion to queue-bench.mjs.
//
//   node scripts/dev/batch-bench.mjs [--browser <exe>] [--out file.json]
//
// For 100 and 500 links (every 25th fails, every 50th is an 8-video playlist):
//   1. pastes them into the link field, then (a separate task, as when typed) presses Ctrl+Enter,
//   2. waits until every row is built and every lookup has answered (mock delays scaled
//      to 2%, so 6 to 46 ms each),
//   3. clicks Download twice in the same task and waits for the batch to be added.
// Recorded: script/layout/style milliseconds and long tasks (> 50 ms) per phase, the most
// probe_url calls in flight at once, enqueue calls and items, and the Queue rows after.
// A last case starts 30 links at full mock speed and presses Stop after 400 ms.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
const browser = args.browser ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const repo = resolve(import.meta.dirname, "..", "..");
const port = 1432;
const debugPort = 9334;

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
const profile = mkdtempSync(join(tmpdir(), "claws-batch-bench-"));
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
let socket;
function cdp(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise((resolvePromise, reject) => pending.set(id, { resolvePromise, reject }));
}
async function evaluate(expression) {
  const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 500));
  return result.result.value;
}
async function metrics() {
  const { metrics: list } = await cdp("Performance.getMetrics");
  const pick = (name) => list.find((m) => m.name === name)?.value ?? 0;
  return { script: pick("ScriptDuration") * 1000, layout: pick("LayoutDuration") * 1000, style: pick("RecalcStyleDuration") * 1000 };
}
const diff = (a, b) => ({ scriptMs: +(b.script - a.script).toFixed(1), layoutMs: +(b.layout - a.layout).toFixed(1), styleMs: +(b.style - a.style).toFixed(1) });
async function until(predicate, timeoutMs = 120000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await evaluate(`Boolean(${predicate})`)) return Date.now() - start;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error(`timed out waiting for ${predicate}`);
}

// Runs inside the page: counts backend calls as they go through the mock.
const pageHelpers = (scale) => `
  window.__clawsMockProbeScale = ${scale};
  window.__calls = { probeInFlight: 0, probeMax: 0, probes: 0, enqueues: 0, enqueuedItems: 0, jobsReturned: 0 };
  {
    const internals = window.__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async (cmd, payload, options) => {
      const c = window.__calls;
      if (cmd === 'probe_url') {
        c.probes++; c.probeInFlight++; c.probeMax = Math.max(c.probeMax, c.probeInFlight);
        try { return await original(cmd, payload, options); } finally { c.probeInFlight--; }
      }
      if (cmd === 'enqueue') {
        c.enqueues++; c.enqueuedItems += payload.items.length;
        const reply = await original(cmd, payload, options);
        c.jobsReturned += reply.jobs.length;
        return reply;
      }
      return original(cmd, payload, options);
    };
  }
  window.__lt = [];
  new PerformanceObserver((list) => list.getEntries().forEach((e) => window.__lt.push(e.duration))).observe({ type: 'longtask' });
  window.__paste = (n, prefix) => {
    const lines = Array.from({ length: n }, (_, i) =>
      i % 25 === 7 ? prefix + 'fail/' + i : i % 50 === 13 ? prefix + 'list/' + i : prefix + 'watch?v=' + i + '&sig=S' + i);
    const field = document.querySelector('.url-input');
    field.value = lines.join('\\n');
    field.dispatchEvent(new Event('input', { bubbles: true }));
    field.focus();
  };
  window.__submit = () => document.querySelector('.url-input').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }));
  true;
`;
const takeLongTasks = () => evaluate("(() => { const d = window.__lt.splice(0); return { count: d.length, longestMs: d.length ? Math.round(Math.max(...d)) : 0 }; })()");
const calls = () => evaluate("JSON.parse(JSON.stringify(window.__calls))");
const settled = "document.querySelector('.batch') && !document.querySelector('.batch-pick[data-status=\"waiting\"], .batch-pick[data-status=\"looking\"]')";

async function openApp(scale) {
  await cdp("Page.navigate", { url: `http://localhost:${port}/` });
  for (let i = 0; i < 100; i++) {
    if (await evaluate("Boolean(document.querySelector('.nav-badge')) && document.readyState === 'complete'").catch(() => false)) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  await evaluate(pageHelpers(scale));
}

const results = [];
try {
  await waitForOutput(vite, /ready in/i);
  socket = new WebSocket(await target());
  await new Promise((r) => socket.addEventListener("open", r, { once: true }));
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    const waiter = pending.get(message.id);
    if (waiter) {
      pending.delete(message.id);
      message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolvePromise(message.result);
    }
  });
  await cdp("Page.enable");
  await cdp("Runtime.enable");
  await cdp("Performance.enable");

  for (const size of [100, 500]) {
    await openApp(0.02);
    await takeLongTasks();
    const mp = await metrics();
    await evaluate(`window.__paste(${size}, 'https://bench.example/n${size}/')`);
    await evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())))");
    const m0 = await metrics();
    const pasteLong = await takeLongTasks();
    const button = await evaluate("document.querySelector('.urlbar .btn-primary').textContent");
    await evaluate("window.__submit()");
    const firstFrameRows = await evaluate("new Promise((r) => requestAnimationFrame(() => r(document.querySelectorAll('.batch-row').length)))");
    const lookupMs = await until(`${settled} && document.querySelectorAll('.batch-row').length === ${size}`);
    await evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())))"); // the counts repaint once a frame
    const m1 = await metrics();
    const lookupLong = await takeLongTasks();
    const c1 = await calls();
    const rows = await evaluate(`(() => {
      const picks = [...document.querySelectorAll('.batch-pick')];
      return { built: picks.length, ready: picks.filter((p) => p.dataset.status === 'ready').length,
        failed: picks.filter((p) => p.dataset.status === 'failed').length,
        selected: picks.filter((p) => p.getAttribute('aria-checked') === 'true').length,
        failedSelectable: picks.filter((p) => p.dataset.status === 'failed' && p.getAttribute('aria-disabled') !== 'true').length,
        retryButtons: document.querySelectorAll('.batch-side .btn').length,
        summary: document.querySelector('.batch .entries-count').textContent,
        button: document.querySelector('.batch .actions .btn-primary').textContent };
    })()`);

    // Two clicks in one task: the second must not add anything.
    await evaluate("(() => { const b = document.querySelector('.batch .actions .btn-primary'); b.click(); b.click(); })()");
    const addMs = await until("window.__calls.enqueues > 0 && !document.querySelector('.batch-pick[aria-checked=\"true\"]')");
    await new Promise((r) => setTimeout(r, 300));
    const m2 = await metrics();
    const addLong = await takeLongTasks();
    const c2 = await calls();
    const afterAdd = await evaluate(`(() => ({
      batchStillShown: Boolean(document.querySelector('.batch')),
      added: document.querySelectorAll('.batch-pick[data-status="added"]').length,
      failedLeft: document.querySelectorAll('.batch-pick[data-status="failed"]').length,
      toast: [...document.querySelectorAll('.toast-text')].map((t) => t.textContent).pop() ?? null,
    }))()`);
    // A second press after everything was added sends nothing more.
    await evaluate("document.querySelector('.batch .actions .btn-primary')?.click()");
    await new Promise((r) => setTimeout(r, 300));
    const c3 = await calls();
    await evaluate("document.dispatchEvent(new KeyboardEvent('keydown', { key: '2', ctrlKey: true, bubbles: true })); new Promise((r) => setTimeout(r, 300))");
    await until("document.querySelectorAll('.jobs .job').length >= 6", 10000);
    // The queue builds new rows 40 a frame; wait for it to settle.
    let queueRows = 0;
    for (let i = 0; i < 100; i++) {
      const next = await evaluate("document.querySelectorAll('.jobs .job').length");
      if (next === queueRows && i > 3) break;
      queueRows = next;
      await new Promise((r) => setTimeout(r, 50));
    }
    const result = {
      links: size,
      paste: { ...diff(mp, m0), longTasks: pasteLong.count, longestTaskMs: pasteLong.longestMs, button },
      lookup: { ...diff(m0, m1), wallMs: lookupMs, longTasks: lookupLong.count, longestTaskMs: lookupLong.longestMs, rowsInFirstFrame: firstFrameRows, probes: c1.probes, maxProbesInFlight: c1.probeMax, rows },
      add: { ...diff(m1, m2), wallMs: addMs, longTasks: addLong.count, longestTaskMs: addLong.longestMs, enqueueCalls: c2.enqueues, itemsSent: c2.enqueuedItems, jobsReturned: c2.jobsReturned, afterAdd },
      secondPressEnqueueCalls: c3.enqueues - c2.enqueues,
      queueRowsAfter: queueRows,
    };
    results.push(result);
    console.log(
      `links=${size}: paste ${JSON.stringify(result.paste)}; lookup ${result.lookup.wallMs} ms wall, ${result.lookup.scriptMs} ms script, ${result.lookup.layoutMs} ms layout, ${result.lookup.styleMs} ms style, ` +
        `${result.lookup.longTasks} long (longest ${result.lookup.longestTaskMs} ms), ${firstFrameRows} rows in first frame, max ${c1.probeMax} probes in flight, ${c1.probes} probes; ` +
        `rows ${JSON.stringify(rows)}; add: ${c2.enqueues} enqueue calls, ${c2.enqueuedItems} items, ${c2.jobsReturned} jobs, ${result.add.longTasks} long (longest ${result.add.longestTaskMs} ms); ` +
        `after ${JSON.stringify(afterAdd)}; second press calls ${result.secondPressEnqueueCalls}; queue rows ${queueRows}`,
    );
  }

  // Stop: 30 links at full mock speed (0.3 to 2.3 s each), Stop after 400 ms.
  await openApp(1);
  await evaluate("window.__paste(30, 'https://bench.example/stop/')");
  await evaluate("window.__submit()");
  await new Promise((r) => setTimeout(r, 400));
  const before = await calls();
  await evaluate("[...document.querySelectorAll('.batch .entries-head .btn')].find((b) => b.textContent === 'Stop').click()");
  await new Promise((r) => setTimeout(r, 2600)); // longer than any lookup still running
  const after = await calls();
  const stop = await evaluate(`(() => ({
    stopped: document.querySelectorAll('.batch-pick[data-status="cancelled"]').length,
    ready: document.querySelectorAll('.batch-pick[data-status="ready"]').length,
    failed: document.querySelectorAll('.batch-pick[data-status="failed"]').length,
    stillLooking: document.querySelectorAll('.batch-pick[data-status="looking"], .batch-pick[data-status="waiting"]').length,
    summary: document.querySelector('.batch .entries-count').textContent,
    focused: document.activeElement?.textContent ?? null,
  }))()`);
  const stopResult = { probesStartedBeforeStop: before.probes, probesStartedAfterStop: after.probes - before.probes, inFlightAfterWait: after.probeInFlight, maxProbesInFlight: after.probeMax, ...stop };
  results.push({ stop: stopResult });
  console.log(`stop: ${JSON.stringify(stopResult)}`);
} finally {
  socket?.close();
  chrome.kill();
  vite.kill();
}
if (args.out) writeFileSync(args.out, JSON.stringify(results, null, 2));
