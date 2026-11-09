#!/usr/bin/env node
// Actual mounted Download view against synthetic dev IPC in an isolated headless Edge profile.
// No Rust backend, user data, live Spotify/YouTube requests or downloads are involved.
// Run: node scripts/dev/spotify-bench.mjs [--browser <exe>] [--out <report.json>]
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, value, i, all) => value.startsWith("--") ? [...pairs, [value.slice(2), all[i + 1]]] : pairs, []));
const repo = resolve(import.meta.dirname, "../..");
const browser = args.browser ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const port = 1447, debugPort = 9357;
const vite = spawn(process.execPath, [join(repo, "node_modules/vite/bin/vite.js"), "--port", String(port), "--strictPort"], { cwd: repo, stdio: ["ignore", "pipe", "pipe"] });
const chrome = spawn(browser, ["--headless=new", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "claws-spotify-bench-"))}`, "--no-first-run", "--window-size=1160,900", "about:blank"], { stdio: "ignore" });
let socket;
let nextId = 1;
const pending = new Map();
const pageErrors = [];
const checks = [];
const check = (name, actual) => { checks.push({ name, pass: Boolean(actual) }); };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const pages = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
      const page = pages.find((item) => item.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await wait(100);
  }
  throw new Error("No headless browser target");
}
function cdp(method, params = {}) {
  const id = nextId++;
  return new Promise((resolvePromise, reject) => {
    pending.set(id, { resolvePromise, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
  return result.result.value;
}
async function until(expression) {
  for (let i = 0; i < 100; i++) {
    if (await evaluate(expression)) return;
    await wait(50);
  }
  throw new Error(`Timed out: ${expression}`);
}
const spotifyUrl = "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT";
const replacementUrl = "https://open.spotify.com/track/0123456789abcdefghijkl";
const lookup = (url) => evaluate(`__s.lookup(${JSON.stringify(url)})`);
const confirm = async () => {
  await until("document.querySelectorAll('input[name=spotify-match]').length === 3");
  await evaluate("__s.choose(1); __s.click('Use this match')");
  await until("Boolean(__s.button('Download'))");
};

try {
  socket = new WebSocket(await target());
  await new Promise((res, rej) => { socket.onopen = res; socket.onerror = rej; });
  socket.onmessage = ({ data }) => {
    const result = JSON.parse(data);
    if (result.id) {
      const promise = pending.get(result.id);
      pending.delete(result.id);
      if (result.error) promise?.reject(new Error(JSON.stringify(result.error)));
      else promise?.resolvePromise(result.result);
    }
    if (result.method === "Runtime.exceptionThrown") pageErrors.push(result.params.exceptionDetails.exception?.description ?? result.params.exceptionDetails.text);
  };
  await cdp("Runtime.enable");
  await cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
  for (let i = 0; i < 100; i++) { try { if ((await fetch(`http://localhost:${port}`)).ok) break; } catch {} await wait(100); }
  await cdp("Page.navigate", { url: `http://localhost:${port}` });
  await until("Boolean(document.querySelector('.url-input'))");
  await evaluate(`(async () => {
    const { store } = await import('/src/store.ts');
    window.__clawsMockProbeScale = 0.001;
    window.__s = {
      store, calls: [], holding: false, pendingEnqueue: null, cacheUrl: null, blockedProbe: null, holdProbe: false, pendingProbe: null,
      wait: (ms) => new Promise(r => setTimeout(r, ms)),
      button: label => [...document.querySelectorAll('.stage button')].find(el => el.textContent.trim() === label),
      click(label) { const el = this.button(label); if (!el) throw new Error('Missing button ' + label); el.click(); },
      choose(index) { const radio = document.querySelectorAll('input[name=spotify-match]')[index]; radio.checked = true; radio.dispatchEvent(new Event('change', { bubbles: true })); },
      lookup(url) { const input = document.querySelector('.url-input'); input.value = url; input.dispatchEvent(new Event('input', { bubbles:true })); input.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter',ctrlKey:url.includes('\\n'),bubbles:true,cancelable:true})); },
      settle(ok) { const item = this.pendingEnqueue; this.pendingEnqueue = null; this.holding = false; ok ? item.resolve(item.run()) : item.reject(new Error('Synthetic enqueue failure')); },
      async nav(route) { store.go(route); await this.wait(35); },
    };
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (cmd, payload) => {
      __s.calls.push({cmd,payload:structuredClone(payload ?? {})});
      if (cmd === 'enqueue' && __s.holding) return new Promise((resolve,reject) => { __s.pendingEnqueue={resolve,reject,run:()=>invoke(cmd,payload)}; });
      if (cmd === 'cached_link' && payload.url === __s.cacheUrl) return Promise.resolve({savedAt:Date.now(),result:__s.cachedResult});
      if (cmd === 'probe_url' && payload.url === __s.cacheUrl) return new Promise(resolve => { __s.blockedProbe=resolve; });
      if (cmd === 'probe_url' && __s.holdProbe) return new Promise(resolve => { __s.pendingProbe=()=>Promise.resolve(invoke(cmd,payload)).then(resolve); });
      return invoke(cmd,payload);
    };
  })()`);

  await lookup(spotifyUrl);
  await until("document.querySelectorAll('input[name=spotify-match]').length === 3");
  check("Spotify routes before cache and generic probe", await evaluate("!__s.calls.some(c=>c.cmd==='cached_link'||c.cmd==='probe_url')"));
  check("No candidate preselected and no Download before confirmation", await evaluate("![...document.querySelectorAll('input[name=spotify-match]')].some(el=>el.checked) && !__s.button('Download')"));
  await evaluate("__s.choose(1)");
  check("Selecting a candidate does not probe or enqueue", await evaluate("!__s.calls.some(c=>c.cmd==='probe_url'||c.cmd==='enqueue')"));
  await evaluate("__s.click('Use this match')");
  await until("Boolean(__s.button('Download'))");
  check("Selected recording receives the sole full probe", await evaluate("JSON.stringify(__s.calls.filter(c=>c.cmd==='probe_url').map(c=>c.payload.url))===JSON.stringify(['https://www.youtube.com/watch?v=abcdefghijk'])"));
  check("Confirmation forces audio and hides Type controls", await evaluate("document.querySelector('[aria-label=Type]').closest('.row').hidden && document.querySelector('[aria-label=Type] [aria-checked=true]').textContent==='Audio Only'"));
  await evaluate("__s.click('FLAC'); document.querySelector('[aria-label=\"Fast Save\"]').click()");
  await evaluate("__s.nav('queue').then(()=>__s.nav('download'))");
  await until("Boolean(__s.button('Download'))");
  check("Navigation retains chosen match and audio options without re-matching", await evaluate("document.querySelector('[aria-label=\"Audio format\"] [aria-checked=true]').textContent==='FLAC' && document.querySelector('[aria-label=\"Fast Save\"]').getAttribute('aria-checked')==='true' && __s.calls.filter(c=>c.cmd==='search_spotify_matches').length===1 && __s.calls.filter(c=>c.cmd==='probe_url').length===1"));
  const outputDir = join(repo, ".test-build/spotify-bench"); mkdirSync(outputDir, { recursive: true });
  const shot = await cdp("Page.captureScreenshot", { format: "png", fromSurface: true });
  writeFileSync(join(outputDir, "confirmation.png"), Buffer.from(shot.data, "base64"));
  await evaluate("document.documentElement.dataset.theme='light'");
  writeFileSync(join(outputDir, "confirmation-light.png"), Buffer.from((await cdp("Page.captureScreenshot", { format: "png", fromSurface: true })).data, "base64"));
  await cdp("Emulation.setDeviceMetricsOverride", { width: 900, height: 1100, deviceScaleFactor: 1, mobile: false });
  writeFileSync(join(outputDir, "confirmation-narrow.png"), Buffer.from((await cdp("Page.captureScreenshot", { format: "png", fromSurface: true })).data, "base64"));
  check("Narrow layout stacks summaries without horizontal page overflow", await evaluate("getComputedStyle(document.querySelector('.spotify-review')).gridTemplateColumns.split(' ').length===1 && document.documentElement.scrollWidth<=innerWidth"));
  await cdp("Emulation.clearDeviceMetricsOverride");

  await evaluate("__s.holding=true; __s.click('Download'); __s.button('Download')?.click()");
  await until("Boolean(__s.pendingEnqueue)");
  check("Double click produces one canonical YouTube audio enqueue with Spotify origin", await evaluate(`(() => { const calls=__s.calls.filter(c=>c.cmd==='enqueue'); const p=calls[0]?.payload; return calls.length===1 && p.items.length===1 && p.items[0].url==='https://www.youtube.com/watch?v=abcdefghijk' && p.items[0].matchOrigin.spotifyUrl===${JSON.stringify(spotifyUrl)} && p.items[0].matchOrigin.kind==='spotifyYouTube' && p.source==='https://www.youtube.com/watch?v=abcdefghijk' && p.options.mode==='audio' && p.options.audioFormat==='flac' && p.options.fastSave===true; })()`));
  await lookup(replacementUrl);
  await until("document.querySelectorAll('input[name=spotify-match]').length === 3");
  await evaluate("__s.settle(true); __s.wait(800)");
  check("Late successful enqueue preserves replacement Spotify session", await evaluate(`document.querySelector('.url-input').value===${JSON.stringify(replacementUrl)} && Boolean(document.querySelector('.spotify-match'))`));
  check("Replacement matching controls become usable after older enqueue settles", await evaluate("!document.querySelector('input[name=spotify-match]').disabled && !document.querySelector('#spotify-query').disabled && !__s.button('Find on YouTube').disabled"));

  await confirm();
  await evaluate("__s.holding=true; __s.click('Download')");
  await until("Boolean(__s.pendingEnqueue)");
  await evaluate("__s.nav('queue').then(()=>__s.nav('download'))");
  check("Returning during enqueue cannot enqueue duplicate", await evaluate("!__s.button('Download') || __s.button('Download').disabled"));
  const countPending = await evaluate("__s.calls.filter(c=>c.cmd==='enqueue').length");
  await evaluate("__s.button('Download')?.click()");
  check("Pending enqueue guard rejects clicks from the remounted view", await evaluate(`__s.calls.filter(c=>c.cmd==='enqueue').length===${countPending}`));
  await evaluate("__s.settle(false); __s.wait(100)");
  check("Failed enqueue after navigation retains confirmed choice for retry", await evaluate("Boolean(__s.button('Download')) && !__s.button('Download').disabled && document.querySelector('.spotify-match').textContent.includes('Source: YouTube match')"));

  // Changing the query removes both the confirmed download controls and its stale handlers.
  await evaluate("__s.oldDownload=__s.button('Download'); const query=document.querySelector('#spotify-query'); query.value='edited sample query'; query.dispatchEvent(new Event('input',{bubbles:true})); __s.oldDownload.click()");
  check("Query edits remove Download and stale buttons cannot enqueue", await evaluate(`!__s.button('Download') && __s.calls.filter(c=>c.cmd==='enqueue').length===${countPending}`));
  await evaluate("__s.click('Find on YouTube')");
  await until("document.querySelectorAll('input[name=spotify-match]').length===3");
  await evaluate("__s.holdProbe=true; __s.choose(2); __s.click('Use this match')");
  await until("Boolean(__s.pendingProbe)");
  await evaluate("__s.nav('queue').then(()=>__s.nav('download'))");
  check("Navigation restores probing as retryable review with no Download", await evaluate("!__s.button('Download') && Boolean(__s.button('Use this match')) && !__s.button('Use this match').disabled && document.querySelector('.spotify-match').textContent.includes('interrupted')"));
  await evaluate("__s.holdProbe=false; __s.pendingProbe(); __s.wait(50)");
  check("Late probe from an unmounted view cannot confirm the restored session", await evaluate("!__s.button('Download')"));
  await evaluate("__s.click('Use this match')");
  await until("Boolean(__s.button('Download'))");
  await evaluate("__s.holding=true; __s.click('Download')");
  await until("Boolean(__s.pendingEnqueue)");
  await evaluate("__s.nav('queue').then(()=>__s.nav('download'))");
  await evaluate("__s.settle(true); __s.wait(800)");
  check("Accepted enqueue clears its remounted owning session", await evaluate("!document.querySelector('.spotify-match') && !__s.button('Download') && __s.store.spotifyMatch===null"));

  await lookup(spotifyUrl); await confirm();
  await evaluate("__s.holding=true; __s.click('Download')");
  await until("Boolean(__s.pendingEnqueue)");
  await lookup("https://example.com/ordinary-replacement");
  await until("Boolean(__s.button('Download')) && !document.querySelector('.spotify-match')");
  check("Ordinary replacement is usable while old matched enqueue is pending", await evaluate("!__s.button('Download').disabled"));
  await evaluate("__s.settle(false); __s.wait(80)");
  check("Late failed matched enqueue preserves the ordinary replacement", await evaluate("!__s.button('Download').disabled && !document.querySelector('.spotify-match') && document.querySelector('.url-input').value==='https://example.com/ordinary-replacement'"));

  // A cached ordinary preview is still checking when a Spotify session replaces it.
  await evaluate(`__s.cacheUrl='https://example.com/cached-check'; __s.cachedResult={kind:'video',url:__s.cacheUrl,title:'Synthetic cached video',uploader:'Sample',thumbnail:null,duration:120,site:'Sample',heights:[],subtitles:[],entries:[],audioAvailable:true}`);
  await lookup("https://example.com/cached-check");
  await until("Boolean(__s.blockedProbe)");
  await lookup(spotifyUrl); await confirm();
  await evaluate("__s.wait(700)");
  check("Spotify confirmation resets prior checking freshness and estimates YouTube source", await evaluate("!document.querySelector('.stage').textContent.includes('Checking for changes') && __s.calls.some(c=>c.cmd==='estimate_download' && c.payload.url==='https://www.youtube.com/watch?v=abcdefghijk' && c.payload.options.mode==='audio')"));
  const oldCalls = await evaluate("__s.calls.length");
  await lookup(`${spotifyUrl}\nhttps://example.com/ordinary-row`);
  await until("document.querySelector('.stage').textContent.includes('individually')");
  await evaluate("__s.wait(200)");
  check("Mixed batch rejects Spotify before generic probe while ordinary row remains usable", await evaluate(`(() => { const calls=__s.calls.slice(${oldCalls}); return !calls.some(c=>c.cmd==='probe_url' && c.payload.url.includes('spotify')) && calls.some(c=>c.cmd==='probe_url' && c.payload.url==='https://example.com/ordinary-row') && document.querySelector('.stage').textContent.includes('individually'); })()`));
  check("Mounted flow has no uncaught browser exceptions", pageErrors.length === 0);
  const report = { scope: "Synthetic dev IPC and actual mounted frontend only", checks, pageErrors, screenshot: join(outputDir, "confirmation.png") };
  if (args.out) writeFileSync(args.out, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  if (checks.some(item => !item.pass)) process.exitCode = 1;
} catch (error) {
  if (socket?.readyState === WebSocket.OPEN) console.error(await evaluate("({stage:document.querySelector('.stage')?.textContent,saved:window.__s?.store.spotifyMatch})").catch(() => null));
  throw error;
} finally {
  socket?.close();
  chrome.kill();
  vite.kill();
}
