// Test driver: talks to the Claws webview over the local DevTools port.
// Usage:
//   node cdp.mjs eval "<js expression>"
//   node cdp.mjs wait "<js predicate>" [timeoutMs]
//   node cdp.mjs shot <out.png> [width height]
import fs from "node:fs";

const [, , cmd, arg1, arg2, arg3] = process.argv;

const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const page = targets.find((t) => t.type === "page" && t.url.includes("localhost:1420"));
if (!page) throw new Error("Claws page target not found");

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});

let nextId = 1;
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
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });

const evaluate = async (expression) => {
  const res = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (res.result?.exceptionDetails) {
    const d = res.result.exceptionDetails;
    throw new Error(d.exception?.description || d.text);
  }
  return res.result?.result?.value;
};

try {
  if (cmd === "eval") {
    const value = await evaluate(arg1);
    console.log(typeof value === "string" ? value : JSON.stringify(value, null, 2));
  } else if (cmd === "wait") {
    const timeout = Number(arg2 || 20000);
    const start = Date.now();
    let ok = false;
    while (Date.now() - start < timeout) {
      if (await evaluate(`Boolean(${arg1})`)) {
        ok = true;
        break;
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    console.log(ok ? `ok after ${Date.now() - start}ms` : "TIMEOUT");
    if (!ok) process.exitCode = 3;
  } else if (cmd === "shot") {
    // Paint an opaque stand-in for the window material so transparent areas read clearly.
    await send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 46, g: 50, b: 68, a: 1 } });
    const params = { format: "png", fromSurface: true };
    const res = await send("Page.captureScreenshot", params);
    fs.writeFileSync(arg1, Buffer.from(res.result.data, "base64"));
    console.log("saved", arg1);
  } else {
    console.log("unknown command");
    process.exitCode = 2;
  }
} finally {
  ws.close();
}
