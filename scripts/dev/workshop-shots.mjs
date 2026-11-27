// Screenshots of the Wallpaper Engine export tracker in every state, from the dev mock only.
// Start the dev server first (`npm run dev`, which serves http://localhost:1420), then:
//   node scripts/dev/workshop-shots.mjs <outDir> [only]
// It launches its own headless Chrome (CHROME=path to override) with a throwaway profile and
// drives the page over the DevTools port. It never captures the desktop, only the page.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const out = process.argv[2];
const only = process.argv[3];
if (!out) throw new Error("usage: workshop-shots.mjs <outDir> [only]");
fs.mkdirSync(out, { recursive: true });

const chrome =
  process.env.CHROME ??
  ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find((p) => fs.existsSync(p));
if (!chrome) throw new Error("No Chrome or Edge found; set CHROME");
const port = 9333;
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "claws-shots-"));
const browser = spawn(chrome, [`--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "--headless=new", "--no-first-run", "--window-size=1040,820", "about:blank"], {
  stdio: "ignore",
});
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function target() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page;
    } catch {
      // not up yet
    }
    await sleep(200);
  }
  throw new Error("Chrome did not start");
}

const page = await target();
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});
let next = 1;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = next++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async (expression) => {
  const res = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (res.result?.exceptionDetails) throw new Error(res.result.exceptionDetails.exception?.description ?? res.result.exceptionDetails.text);
  return res.result?.result?.value;
};
const waitFor = async (predicate, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await evaluate(`Boolean(${predicate})`)) return true;
    await sleep(120);
  }
  throw new Error(`timed out waiting for ${predicate}`);
};

const ITEM = (id) => `https://steamcommunity.com/sharedfiles/filedetails/?id=${id}`;
const COPY = "1000000015"; // a video, H.264 in an MP4: copied
const HEVC = "1000000055"; // a video that has to be converted
const SCENE = "1000000017"; // a scene: its project is zipped

/** Each state of the tracker, and how to reach it in the sample app. */
const states = [
  { name: "ready-copy-estimate", id: COPY, query: "", reach: "estimate" },
  { name: "ready-convert-no-estimate", id: HEVC, query: "", reach: "estimate" },
  { name: "checking", id: COPY, query: "hold=checking", reach: "start" },
  { name: "copying", id: COPY, query: "hold=copying:3", reach: "start" },
  { name: "finishing", id: COPY, query: "hold=finishing", reach: "start", text: "Finishing" },
  { name: "converting-no-time-left", id: HEVC, query: "hold=converting:2", reach: "start" },
  { name: "converting-time-left", id: HEVC, query: "hold=converting:5", reach: "start" },
  { name: "zipping", id: SCENE, query: "hold=zipping:4", reach: "start" },
  { name: "saved", id: COPY, query: "", reach: "saved" },
  { name: "failed", id: COPY, query: "fail=1", reach: "failed" },
  { name: "cancelled", id: COPY, query: "hold=copying:3", reach: "cancelled" },
];

/** The looks each state is shot in: theme, Transparency (glass intensity) and Reduce motion. */
const looks = [
  { tag: "light", dark: false, glass: 0.7, reduce: false },
  { tag: "dark", dark: true, glass: 0.7, reduce: false },
];
/** A few states get the full matrix. */
const extra = new Set(["copying", "saved", "failed", "converting-time-left"]);
const extraLooks = [
  { tag: "light-t0", dark: false, glass: 0, reduce: false },
  { tag: "light-t100", dark: false, glass: 1, reduce: false },
  { tag: "dark-t0", dark: true, glass: 0, reduce: false },
  { tag: "dark-t100", dark: true, glass: 1, reduce: false },
  { tag: "light-reduce-motion", dark: false, glass: 0.7, reduce: true },
  { tag: "dark-reduce-motion", dark: true, glass: 0.7, reduce: true },
];

async function shoot(state, look) {
  await send("Emulation.setDeviceMetricsOverride", { width: 1040, height: 820, deviceScaleFactor: 1, mobile: false });
  await send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-color-scheme", value: look.dark ? "dark" : "light" },
      { name: "prefers-reduced-motion", value: look.reduce ? "reduce" : "no-preference" },
    ],
  });
  const query = ["installed=1", state.query].filter(Boolean).join("&");
  await send("Page.navigate", { url: `http://localhost:1420/?${query}` });
  await waitFor('document.querySelector(".link-input, input[type=text], textarea")');
  await sleep(400);
  await evaluate(`document.documentElement.style.setProperty("--glass-i", "${look.glass}"); document.documentElement.dataset.theme = "${look.dark ? "dark" : "light"}";`);
  // Paste the link the way a user would.
  await evaluate(`(() => { const dt = new DataTransfer(); dt.setData("text", ${JSON.stringify(ITEM(state.id))}); document.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true })); })()`);
  await waitFor('document.querySelector(".workshop-panel")');
  await waitFor('document.querySelector(".workshop-panel .actions .btn-primary:not([disabled])")');
  await sleep(500);
  if (state.reach === "estimate") {
    await waitFor('document.querySelector(".workshop-estimate") && document.querySelector(".workshop-estimate").textContent.length > 0');
  } else {
    await evaluate('document.querySelector(".workshop-panel .actions .btn-primary").click()');
    if (state.reach === "start") await waitFor('document.querySelector(".workshop-progress:not([hidden]) .job-status") && document.querySelector(".workshop-progress .job-status").textContent.length > 0');
    if (state.reach === "saved") await waitFor('document.querySelector(".workshop-progress[data-status=saved]")', 40000);
    if (state.reach === "failed") await waitFor('document.querySelector(".workshop-progress[data-status=failed]")', 40000);
    if (state.reach === "cancelled") {
      await waitFor('document.querySelector(".workshop-progress .job-status")?.textContent.startsWith("Copying 1.9")', 40000);
      await evaluate('[...document.querySelectorAll(".workshop-panel .actions .btn")].find((b) => b.textContent.trim() === "Cancel").click()');
      await waitFor('document.querySelector(".workshop-progress[data-status=cancelled]")');
    }
    if (state.reach === "start") {
      // Wait for the sample export to reach the held step.
      if (state.text) await waitFor(`document.querySelector(".workshop-progress .job-status")?.textContent.startsWith(${JSON.stringify(state.text)})`, 40000);
      else await sleep(state.query.includes(":") ? 5200 : 1800);
    }
    // A finished state is shot after its completion movement has settled.
    await sleep(600);
  }
  const text = await evaluate('document.querySelector(".workshop-progress:not([hidden]) .job-status")?.textContent ?? document.querySelector(".workshop-estimate")?.textContent ?? ""');
  const res = await send("Page.captureScreenshot", { format: "png", fromSurface: true });
  const file = path.join(out, `${state.name}--${look.tag}.png`);
  fs.writeFileSync(file, Buffer.from(res.result.data, "base64"));
  console.log(`${state.name} (${look.tag}): ${JSON.stringify(text)}`);
}

try {
  await send("Page.enable");
  for (const state of states) {
    if (only && state.name !== only) continue;
    for (const look of extra.has(state.name) ? [...looks, ...extraLooks] : looks) await shoot(state, look);
  }
} finally {
  ws.close();
  browser.kill();
  await sleep(300);
  fs.rmSync(profile, { recursive: true, force: true });
}
