#!/usr/bin/env node
// Measures each motion effect in the real frontend, running in a headless Edge/Chrome
// against the dev mock (npm run dev). No backend, no live sites. Companion to queue-bench.mjs.
//
//   node scripts/dev/motion-bench.mjs [--browser <exe>] [--out file.json]
//
// Runs twice: with motion, and with prefers-reduced-motion: reduce emulated over CDP.
// For each effect (link lookup settle, Type segmented indicator, an idle Queue, queue row
// entry, row removal, bulk removal, progress smoothing, completion check, file-drag pickup) it records the
// script/layout/style milliseconds (CDP Performance.getMetrics), long tasks (> 50 ms),
// frames slower than 25 ms and the animations running, over a fixed window after the
// trigger. It also records examples (the progress fill's scaleX each frame; where a
// removed row sits while it collapses) and checks focus, rapid repeats (double and held
// Enter on a row's buttons, as trusted CDP key events), a hidden document and leaving the
// view mid-animation (every Element.animate call is tracked, since document.getAnimations()
// cannot see animations on detached elements).
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, value, i, all) => (value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs), []),
);
const browser = args.browser ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const repo = resolve(import.meta.dirname, "..", "..");
const port = 1433;
const debugPort = 9335;

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
const profile = mkdtempSync(join(tmpdir(), "claws-motion-bench-"));
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
  const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
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

// Runs inside the page. Emits through the mock's event bus, exactly as the backend would.
const pageHelpers = `
  window.__m = {
    emit: (event, payload) => window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event, payload }),
    frame: () => new Promise((r) => requestAnimationFrame(() => r())),
    wait: (ms) => new Promise((r) => setTimeout(r, ms)),
    revision: 5000,
    jobs: new Map(),
    job(i, change = {}) {
      const id = 'm' + i;
      const base = this.jobs.get(id) ?? { id, url: 'https://example.com/m/' + i, title: 'Motion job ' + i, thumbnail: null, uploader: 'Bench', duration: 60,
        options: { mode: 'video', maxHeight: null, videoPreset: 'compatible', audioFormat: 'mp3', subtitleLangs: [], autoSubs: false, embedSubs: false, fastSave: false, transcriptFormat: 'txt' },
        status: 'queued', progress: 0, progressKnown: false, speed: null, eta: null, filePath: null, error: null, createdAt: 1e13 + i,
        downloadedBytes: null, totalBytes: null, progressEstimated: false };
      const job = { ...base, ...change, revision: ++this.revision };
      this.jobs.set(id, job);
      return job;
    },
    update(i, change) {
      const j = this.job(i, change);
      return this.emit('job-update', { schemaVersion: 1, jobId: j.id, revision: j.revision, job: j });
    },
    add(from, n) {
      const jobs = Array.from({ length: n }, (_, k) => this.job(from + k));
      return this.emit('jobs-added', { schemaVersion: 1, jobs: jobs.map((j) => ({ schemaVersion: 1, jobId: j.id, revision: j.revision, job: j })) });
    },
    remove(i) {
      this.jobs.delete('m' + i);
      return this.emit('job-removed', { schemaVersion: 1, jobId: 'm' + i, revision: ++this.revision });
    },
    row(i) { return [...document.querySelectorAll('.jobs .job')].find((el) => el.querySelector('.job-title')?.textContent === 'Motion job ' + i) ?? null; },
    scaleX(el) {
      const t = getComputedStyle(el).transform;
      if (t === 'none') return 1;
      const m = /matrix\\(([^,]+)/.exec(t);
      return m ? Math.round(+m[1] * 1000) / 1000 : t;
    },
    // Scripted movements only (Element.animate), not CSS transitions or the sheen's keyframes.
    scripted(selector) {
      return document.getAnimations().filter((a) => a.playState === 'running' && !(a instanceof CSSTransition) && !(a instanceof CSSAnimation) && a.effect?.target?.matches?.(selector)).length;
    },
    running(selector) {
      return document.getAnimations().filter((a) => a.playState === 'running' && (!selector || a.effect?.target?.closest?.(selector))).length;
    },
    // Frame pacing over a window: every frame's gap, and optionally a probe per frame.
    async sample(ms, probe) {
      const gaps = [], samples = [];
      const start = performance.now();
      let last = start;
      while (performance.now() - start < ms) {
        await this.frame();
        const now = performance.now();
        gaps.push(now - last);
        last = now;
        if (probe) samples.push([Math.round(now - start), probe()]);
      }
      return { frames: gaps.length, maxFrameMs: Math.round(Math.max(...gaps)), slowFrames: gaps.filter((g) => g > 25).length, samples };
    },
    key(target, key, extra = {}) {
      target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...extra }));
    },
  };
  // Every scripted animation, including ones whose element has since left the document.
  window.__anims = [];
  if (!Element.prototype.__clawsSpy) {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (...a) { const anim = animate.apply(this, a); window.__anims.push(anim); return anim; };
    Element.prototype.__clawsSpy = true;
  }
  window.__lt = [];
  new PerformanceObserver((list) => list.getEntries().forEach((e) => window.__lt.push(e.duration))).observe({ type: 'longtask' });
  true;
`;

/** Runs `trigger` (page code), then measures `ms` of frames after it. */
async function effect(trigger, ms, probe = null) {
  await takeLongTasks();
  const m0 = await metrics();
  const sampled = await evaluate(`(async () => { const before = window.__m.running(); ${trigger}; const runningAfterTrigger = window.__m.running(); const s = await window.__m.sample(${ms}${probe ? `, () => (${probe})` : ""}); return { ...s, runningBefore: before, runningAfterTrigger }; })()`);
  const m1 = await metrics();
  const long = await takeLongTasks();
  return { ...diff(m0, m1), longTasks: long.count, longestTaskMs: long.longestMs, ...sampled };
}

async function openApp(reduced) {
  await cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: reduced ? "reduce" : "no-preference" }] });
  await cdp("Page.navigate", { url: `http://localhost:${port}/` });
  for (let i = 0; i < 100; i++) {
    if (await evaluate("Boolean(document.querySelector('.nav-badge')) && document.readyState === 'complete'").catch(() => false)) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  await evaluate(pageHelpers);
  await evaluate("new Promise((r) => setTimeout(r, 400))");
}
/** Trusted Enter key events, as a keyboard sends them: "press" (down and up), or "down", "repeat" (auto-repeat while held) and "up". */
async function press(step = "press") {
  const key = { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 };
  if (step !== "up") await cdp("Input.dispatchKeyEvent", { type: "keyDown", text: "\r", autoRepeat: step === "repeat", ...key });
  if (step === "press" || step === "up") await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...key });
}
const go = (n) => evaluate(`window.__m.key(document, '${n}', { ctrlKey: true }); new Promise((r) => setTimeout(r, 500))`);

async function run(reduced) {
  const out = { reducedMotion: reduced };
  await openApp(reduced);
  pageErrors.length = 0;

  // 1. Link lookup settling into its details.
  await evaluate("window.__clawsMockProbeScale = 0.1");
  const lookup = (url) => `{ const f = document.querySelector('.url-input'); f.focus(); f.value = '${url}'; f.dispatchEvent(new Event('input', { bubbles: true })); window.__m.key(f, 'Enter'); }`;
  const resultShown = "document.querySelector('.stage .segmented')";
  await evaluate(`(async () => { ${lookup("https://example.com/watch?v=warm")}; for (let i = 0; i < 200 && !${resultShown}; i++) await window.__m.frame(); })()`);
  await evaluate(`(async () => { ${lookup("https://example.com/watch?v=motion1")}; for (let i = 0; i < 200 && !document.querySelector('.stage .loading'); i++) await window.__m.frame(); })()`);
  out.settleIn = await effect(`for (let i = 0; i < 300 && !${resultShown}; i++) await window.__m.frame()`, 700);
  out.settleIn.focusAfter = await evaluate("document.activeElement?.className ?? null");
  // Rapid repeats: four lookups one after another; only the last one's details remain.
  await evaluate(`(async () => { for (const n of [11, 12, 13, 14]) { ${lookup("https://example.com/watch?v=rapid' + n + '")}; await window.__m.wait(15); } })()`);
  await evaluate(`(async () => { await window.__m.wait(600); for (let i = 0; i < 300 && !${resultShown}; i++) await window.__m.frame(); await window.__m.wait(800); })()`);
  out.settleIn.rapid = await evaluate("({ results: document.querySelectorAll('.stage .segmented').length, stageAnimationsLeft: window.__m.running('.stage'), field: document.querySelector('.url-input').value })");

  // 2. Type segmented indicator, driven from the keyboard.
  const seg = "document.querySelector('.stage .segmented[aria-label=\"Type\"]')";
  await evaluate(`${seg}.querySelector('[aria-checked=\"true\"]').focus()`);
  out.segmented = await effect(`window.__m.key(document.activeElement, 'ArrowRight')`, 600);
  out.segmented.focus = await evaluate(`({ onSegment: document.activeElement?.classList.contains('seg-item') ?? false, checked: ${seg}.querySelector('[aria-checked=\"true\"]')?.textContent })`);
  out.segmented.rapid = await evaluate(`(async () => {
    for (let i = 0; i < 7; i++) window.__m.key(document.activeElement, i % 2 ? 'ArrowLeft' : 'ArrowRight');
    await window.__m.wait(700);
    const el = ${seg};
    const checked = [...el.querySelectorAll('.seg-item')].findIndex((b) => b.getAttribute('aria-checked') === 'true');
    return { indexMatchesThumb: String(checked) === el.style.getPropertyValue('--i'), focusOnChecked: document.activeElement?.getAttribute('aria-checked') === 'true' };
  })()`);

  // 3. Queue: an idle second with the mock's rows (one downloading, one processing with the
  // indeterminate sheen), then new rows arriving (a small batch animates; a large one does not,
  // including the rows of its second frame).
  await go(2);
  out.queueIdle = await effect("", 1000);
  out.rowEntry = await effect("await window.__m.add(0, 12); await window.__m.frame(); await window.__m.frame(); window.__smallEntering = window.__m.scripted('.job')", 700);
  out.rowEntry.animatingAfterTwoFrames = await evaluate("window.__smallEntering");
  out.rowEntry.rows = await evaluate("document.querySelectorAll('.jobs .job').length");
  out.rowEntryLarge = await effect("await window.__m.add(100, 50); for (let i = 0; i < 3; i++) await window.__m.frame(); window.__largeEntering = window.__m.scripted('.job')", 700);
  out.rowEntryLarge.rows = await evaluate("document.querySelectorAll('.jobs .job').length");
  out.rowEntryLarge.animatingAfterThreeFrames = await evaluate("window.__largeEntering");

  // 4. Removal: where the leaving row sits, frame by frame, and whether its neighbours jump.
  out.removal = await effect(
    "window.__leaving = window.__m.row(1); window.__below = window.__m.row(2); window.__belowTop = window.__below.getBoundingClientRect().top; await window.__m.remove(1)",
    450,
    "({ leavingIndex: window.__leaving.isConnected ? [...window.__leaving.parentNode.children].indexOf(window.__leaving) : null, belowShiftPx: Math.round(window.__below.getBoundingClientRect().top - window.__belowTop) })",
  );
  out.removal.order = await evaluate("[...document.querySelectorAll('.jobs .job .job-title')].map((t) => t.textContent).filter((t) => t.startsWith('Motion job')).slice(0, 4)");
  out.removal.rapid = await evaluate(`(async () => {
    const leaving = [2, 3, 4].map((i) => window.__m.row(i));
    await window.__m.remove(2); await window.__m.remove(3); await window.__m.remove(4);
    await window.__m.frame(); await window.__m.frame();
    const firstIndexes = leaving.map((el) => (el.isConnected ? [...el.parentNode.children].indexOf(el) : null));
    await window.__m.wait(600);
    return { leavingIndexesAfterTwoFrames: firstIndexes, gone: leaving.every((el) => !el.isConnected), next: [...document.querySelectorAll('.jobs .job .job-title')].map((t) => t.textContent).filter((t) => t.startsWith('Motion job'))[0] };
  })()`);

  // 4b. Many rows leaving at once (as Clear Finished does): no pile of height animations.
  out.bulkRemoval = await effect("for (let i = 100; i < 120; i++) window.__m.remove(i); await window.__m.frame(); await window.__m.frame(); window.__bulkLeaving = window.__m.scripted('.job')", 500);
  out.bulkRemoval.animatingAfterTwoFrames = await evaluate("window.__bulkLeaving");
  out.bulkRemoval.rows = await evaluate("document.querySelectorAll('.jobs .job').length");

  // 5. Progress: a start, one forward update (sampled), steady updates at the backend's
  // pace, then a failure and retry (sampled).
  const fill = (i) => `window.__m.row(${i}).querySelector('.bar-fill')`;
  await evaluate(`(async () => { await window.__m.update(5, { status: 'downloading', progress: 0.4, progressKnown: true }); await window.__m.wait(900); })()`);
  out.progressStep = await effect(`await window.__m.update(5, { progress: 0.5 })`, 800, `window.__m.scaleX(${fill(5)})`);
  out.progressStream = await effect(
    `for (let r = 1; r <= 20; r++) { for (let i = 5; i < 13; i++) await window.__m.update(i, { status: 'downloading', progress: Math.min(0.95, 0.4 + r * 0.025), progressKnown: true, speed: 5e6 }); await window.__m.wait(150); }`,
    300,
  );
  await evaluate(`(async () => { await window.__m.update(5, { status: 'failed', progress: 0.8, error: 'Sample failure' }); await window.__m.wait(900); })()`);
  out.progressRetry = await effect(`await window.__m.update(5, { status: 'queued', progress: 0, progressKnown: false, error: null }); await window.__m.update(5, { status: 'downloading', progress: 0.02, progressKnown: true })`, 700, `window.__m.scaleX(${fill(5)})`);
  // Unknown size: the sheen, and how much of it is on the track over one 1.4 s pass (a
  // static segment with reduced motion). Its left edge travels from off-track to the end.
  out.progressIndeterminate = await evaluate(`(async () => {
    await window.__m.update(14, { status: 'downloading', progress: 0, progressKnown: false });
    await window.__m.frame(); await window.__m.frame();
    const bar = window.__m.row(14).querySelector('.bar'), fill = bar.querySelector('.bar-fill');
    const seen = [];
    for (let t = performance.now(); performance.now() - t < 1500; ) {
      await window.__m.frame();
      const b = bar.getBoundingClientRect(), f = fill.getBoundingClientRect();
      seen.push({ visible: Math.max(0, Math.min(b.right, f.right) - Math.max(b.left, f.left)) / b.width, left: (f.left - b.left) / b.width });
    }
    const r = (v) => Math.round(v * 100) / 100;
    return { indeterminate: bar.classList.contains('indeterminate'), visibleMin: r(Math.min(...seen.map((x) => x.visible))), visibleMax: r(Math.max(...seen.map((x) => x.visible))), leftMin: r(Math.min(...seen.map((x) => x.left))), leftMax: r(Math.max(...seen.map((x) => x.left))) };
  })()`);

  // 6. Completion: state and check in the same frame as the event; the fill sampled after.
  await evaluate(`(async () => { await window.__m.update(6, { status: 'downloading', progress: 0.97, progressKnown: true }); await window.__m.wait(900); })()`);
  out.completion = await effect(
    `await window.__m.update(6, { status: 'completed', progress: 1, progressKnown: true, filePath: 'C:\\\\Users\\\\Sample\\\\Downloads\\\\Claws\\\\m6.mp4' }); var row = window.__m.row(6); window.__completedAtOnce = { status: row.dataset.status, check: Boolean(row.querySelector('.job-status svg')) }`,
    500,
    `window.__m.scaleX(${fill(6)})`,
  );
  out.completion.sameTask = await evaluate("window.__completedAtOnce");
  out.completion.statusAnimations = await evaluate("(async () => { await window.__m.update(7, { status: 'completed', progress: 1, progressKnown: true, filePath: 'C:\\\\x\\\\m7.mp4' }); return window.__m.scripted('.job-status'); })()");
  out.completion.checksAfterRepaints = await evaluate("(async () => { for (let i = 0; i < 5; i++) await window.__m.update(7, { speed: null }); return window.__m.row(7).querySelectorAll('.job-status svg').length; })()");

  // 7. File-drag pickup: feedback lasts exactly as long as the (stubbed) native call.
  await evaluate(`{
    const internals = window.__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = (cmd, payload, options) => cmd === 'drag_download' ? new Promise((r) => setTimeout(() => r('cancelled'), 400)) : original(cmd, payload, options);
  }`);
  out.dragPickup = await effect(
    `const b = window.__m.row(6).querySelector('.file-drag'); const r = b.getBoundingClientRect();
     b.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, buttons: 1, pointerType: 'mouse', clientX: r.x + 4, clientY: r.y + 4 }));
     document.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, button: -1, buttons: 1, pointerType: 'mouse', clientX: r.x + 20, clientY: r.y + 4 }));
     window.__dragActive = b.classList.contains('file-drag-active')`,
    600,
  );
  out.dragPickup.activeDuringCall = await evaluate("window.__dragActive");
  out.dragPickup.activeAfterCall = await evaluate("window.__m.row(6).querySelector('.file-drag').classList.contains('file-drag-active')");

  // 8. Keyboard focus on a row's button through arrivals, removals, progress and a status change.
  out.focus = await evaluate(`(async () => {
    await window.__m.update(8, { status: 'downloading', progress: 0.3, progressKnown: true });
    await window.__m.wait(100);
    const pause = window.__m.row(8).querySelector('button[title="Pause"]');
    pause.focus();
    await window.__m.add(200, 3); await window.__m.remove(9); await window.__m.update(8, { progress: 0.35 });
    await window.__m.wait(500);
    const keptThroughChanges = document.activeElement === pause;
    await window.__m.update(8, { status: 'paused' });
    await window.__m.frame();
    const a = document.activeElement;
    return { keptThroughChanges, afterStatusChange: a === document.body ? 'body' : (a.getAttribute('title') ?? a.textContent), insideRow: window.__m.row(8).contains(a) };
  })()`);

  // 8b. Rapid repeated presses on a row's buttons, as trusted key events. The mock's
  // commands are stubbed to answer at once with the new status, the fastest a backend could.
  await evaluate(`{
    const internals = window.__TAURI_INTERNALS__;
    const original = internals.__benchOriginal ?? internals.invoke;
    internals.__benchOriginal = original;
    window.__calls = [];
    // Per command, how long after the command answers its status change arrives (a running
    // download's pause answers at once; the status follows when the process exits).
    window.__statusDelay = {};
    const status = { pause_job: 'paused', resume_job: 'downloading', cancel_job: 'cancelled' };
    internals.invoke = (cmd, payload, options) => {
      if (cmd in status || cmd === 'remove_job') {
        window.__calls.push(cmd);
        const i = +String(payload.id).slice(1);
        if (cmd === 'remove_job') return window.__m.remove(i).then(() => null);
        const delay = window.__statusDelay[cmd] ?? 0;
        if (delay) { setTimeout(() => window.__m.update(i, { status: status[cmd] }), delay); return Promise.resolve(null); }
        return window.__m.update(i, { status: status[cmd] }).then(() => null);
      }
      return original(cmd, payload, options);
    };
  }`);
  const pressCase = async (i, title, presses) => {
    await evaluate(`(async () => { await window.__m.update(${i}, { status: 'downloading', progress: 0.3, progressKnown: true }); await window.__m.wait(100); window.__calls.length = 0; window.__m.row(${i}).querySelector('button[title="${title}"]').focus(); })()`);
    for (const p of presses) {
      if (p === "wait") await evaluate("window.__m.wait(60)");
      else await press(p);
    }
    await evaluate("window.__m.wait(300)");
    return evaluate(`(() => { const a = document.activeElement; return { calls: [...window.__calls], status: window.__m.row(${i}).dataset.status, focus: a === document.body ? 'body' : (a.getAttribute('title') ?? a.tagName) }; })()`);
  };
  out.rapidPress = {
    // Enter, Enter about 60 ms apart: the second lands on Resume / Try again and must do nothing.
    pauseTwice: await pressCase(120, "Pause", ["press", "wait", "press"]),
    cancelTwice: await pressCase(121, "Cancel download", ["press", "wait", "press"]),
    // Enter held down past the guard: auto-repeat keydowns must not act on the button that takes focus.
    pauseHeld: await pressCase(122, "Pause", ["down", "wait", ...Array.from({ length: 24 }, (_, k) => (k % 2 ? "wait" : "repeat")), "up"]),
  };
  // A guarded button works again once the guard is over (600 ms): Resume, still focused on the held-Enter row.
  out.rapidPress.resumeLater = await evaluate(`(async () => {
    await window.__m.wait(700);
    window.__calls.length = 0;
    return true;
  })()`).then(async () => {
    await press();
    await evaluate("window.__m.wait(200)");
    return evaluate("({ calls: [...window.__calls], status: window.__m.row(122).dataset.status })");
  });
  // A press whose status change comes later than an earlier press's 1.5 s fallback timer:
  // Resume (answered at once), then 1 s later Pause, whose "paused" arrives 1.2 s after that.
  // Focus must still land on Resume and the guard must still hold off a second Enter.
  out.rapidPress.latePause = await evaluate(`(async () => {
    await window.__m.update(125, { status: 'paused', progress: 0.3, progressKnown: true });
    await window.__m.wait(800);
    window.__calls.length = 0;
    window.__m.row(125).querySelector('button[title="Resume"]').focus();
    return true;
  })()`).then(async () => {
    await press();
    await evaluate("window.__m.wait(1000)");
    const beforePause = await evaluate("document.activeElement?.getAttribute('title') ?? 'body'");
    await evaluate("window.__statusDelay.pause_job = 1200; true");
    await press();
    // Past the Resume press's fallback timer (about 1.5 s after it), before "paused" arrives.
    await evaluate("window.__m.wait(1300)");
    await evaluate("(async () => { for (let i = 0; i < 200 && window.__m.row(125).dataset.status !== 'paused'; i++) await window.__m.frame(); await window.__m.frame(); })()");
    const afterPaused = await evaluate("(() => { const a = document.activeElement; return a === document.body ? 'body' : (a.getAttribute('title') ?? a.tagName); })()");
    await evaluate("window.__m.wait(60)");
    await press();
    await evaluate("window.__m.wait(300)");
    const result = await evaluate("({ calls: [...window.__calls], status: window.__m.row(125).dataset.status })");
    await evaluate("window.__statusDelay = {}; true");
    return { focusBeforePause: beforePause, focusAfterPaused: afterPaused, ...result };
  });
  // Removing the row that holds focus: focus moves to the neighbour (never to its Remove),
  // the leaving row is inert, and a second Enter does not act on the neighbour.
  out.removalFocus = await evaluate(`(async () => {
    for (const i of [123, 124]) await window.__m.update(i, { status: 'failed', progress: 0.5, error: 'Sample failure' });
    await window.__m.wait(100);
    window.__leavingRow = window.__m.row(123);
    window.__leavingRow.querySelector('button[title="Remove from list"]').focus();
    window.__calls.length = 0;
    return true;
  })()`).then(async () => {
    await press();
    await evaluate("window.__m.frame().then(() => window.__m.frame())");
    const during = await evaluate(`(() => { const a = document.activeElement, row = window.__leavingRow; return { leavingInert: row.isConnected ? row.inert : 'gone', leavingFocusable: row.isConnected ? row.querySelectorAll('button').length > 0 && !row.inert : false, focus: a === document.body ? 'body' : a.getAttribute('title'), focusInNeighbour: window.__m.row(124)?.contains(a) ?? false }; })()`);
    await evaluate("window.__m.wait(60)");
    await press();
    await evaluate("window.__m.wait(400)");
    const after = await evaluate("({ calls: [...window.__calls], neighbourStatus: window.__m.row(124)?.dataset.status ?? null })");
    return { ...during, ...after };
  });
  await evaluate("{ const i = window.__TAURI_INTERNALS__; i.invoke = i.__benchOriginal; }");

  // 9. A hidden window: nothing new animates; state is still drawn at once.
  out.hidden = await evaluate(`(async () => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
    try {
      await window.__m.update(10, { status: 'downloading', progress: 0.2, progressKnown: true });
      await window.__m.wait(900);
      const before = window.__m.running();
      await window.__m.add(300, 3);
      await window.__m.frame(); await window.__m.frame();
      const rowAnimations = window.__m.scripted('.job');
      await window.__m.update(10, { progress: 0.6 });
      const snapped = window.__m.row(10).querySelector('.bar').classList.contains('bar-snap');
      await window.__m.update(11, { status: 'completed', progress: 1, progressKnown: true, filePath: 'C:\\\\x\\\\m11.mp4' });
      return { runningBefore: before, rowAnimations, progressSnapped: snapped, statusAnimations: window.__m.scripted('.job-status'), check: Boolean(window.__m.row(11).querySelector('.job-status svg')) };
    } finally { delete document.hidden; }
  })()`);

  // 10. Leaving the view mid-animation: nothing keeps running for detached rows, no errors.
  // Counted from every Element.animate call (window.__anims), which still holds animations
  // whose element has left the document; document.getAnimations() cannot see those.
  out.disposal = await evaluate(`(async () => {
    window.__anims.length = 0;
    await window.__m.add(400, 5);
    await window.__m.update(402, { status: 'completed', progress: 1, progressKnown: true, filePath: 'C:\\\\x\\\\m402.mp4' });
    await window.__m.frame(); await window.__m.frame();
    await window.__m.remove(401);
    await window.__m.frame(); await window.__m.frame();
    const live = () => window.__anims.filter((a) => a.playState === 'running');
    const runningBefore = live().length;
    const collapsingBefore = live().filter((a) => a.effect?.getKeyframes?.().some((k) => k.height === '0px' && k.computedOffset === 1)).length;
    window.__m.key(document, '1', { ctrlKey: true });
    await window.__m.frame(); await window.__m.frame();
    const detached = live().filter((a) => a.effect?.target && !a.effect.target.isConnected).length;
    await window.__m.wait(800);
    return { runningBefore, collapsingBefore, runningOnDetachedRows: detached, runningAfter: document.getAnimations().filter((a) => a.playState === 'running').length };
  })()`);
  // 11. The first row into an empty Queue: its arrival grows to the row's real height.
  out.firstRow = await evaluate(`(async () => {
    for (const id of ['a1', 'a2', 'a3', 'a4', 'a5', 'a6']) await window.__m.emit('job-removed', { schemaVersion: 1, jobId: id, revision: ++window.__m.revision });
    for (const id of [...window.__m.jobs.keys()]) await window.__m.remove(+id.slice(1));
    window.__m.key(document, '2', { ctrlKey: true });
    await window.__m.wait(600);
    const emptyShown = document.querySelector('.jobs')?.hidden ?? true;
    await window.__m.add(900, 1);
    const heights = [];
    for (let t = performance.now(); performance.now() - t < 450; ) {
      await window.__m.frame();
      const row = window.__m.row(900);
      heights.push([Math.round(performance.now() - t), row ? Math.round(row.getBoundingClientRect().height) : null]);
    }
    return { emptyShown, heights };
  })()`);
  out.pageErrors = [...pageErrors];
  return out;
}

const results = [];
try {
  await waitForOutput(vite, /ready in/i);
  socket = new WebSocket(await target());
  await new Promise((r) => socket.addEventListener("open", r, { once: true }));
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.method === "Runtime.exceptionThrown") pageErrors.push(message.params.exceptionDetails.exception?.description?.slice(0, 300) ?? message.params.exceptionDetails.text);
    if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") pageErrors.push(message.params.args.map((a) => a.value ?? a.description).join(" ").slice(0, 300));
    const waiter = pending.get(message.id);
    if (waiter) {
      pending.delete(message.id);
      message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolvePromise(message.result);
    }
  });
  await cdp("Page.enable");
  await cdp("Runtime.enable");
  await cdp("Performance.enable");
  for (const reduced of [false, true]) {
    const result = await run(reduced);
    results.push(result);
    console.log(JSON.stringify(result, (key, value) => (key === "samples" ? `${value.length} samples` : value)));
  }
} finally {
  socket?.close();
  chrome.kill();
  vite.kill();
}
if (args.out) writeFileSync(args.out, JSON.stringify(results, null, 2));
