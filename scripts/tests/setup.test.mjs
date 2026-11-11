import assert from "node:assert/strict";
import test from "node:test";

import { SETUP_STEPS, missingTools, stepState, toolReady } from "../../.test-build/setup.js";

const tool = { path: "C:\\tools\\x.exe", version: "1", managed: true, fastStart: false };

test("a clean PC needs all three tools, in install order", () => {
  assert.deepEqual(missingTools({ ytdlp: null, ffmpeg: null, jsRuntime: null }), ["ytdlp", "ffmpeg", "deno"]);
  assert.deepEqual(SETUP_STEPS.map((step) => step.tool), ["ytdlp", "ffmpeg", "deno"]);
});

test("a tool found anywhere is ready, and Node.js counts as the JavaScript runtime", () => {
  const tools = { ytdlp: tool, ffmpeg: null, jsRuntime: "Node.js" };
  assert.deepEqual(missingTools(tools), ["ffmpeg"]);
  assert.equal(toolReady(tools, "deno"), true);
  assert.equal(toolReady(tools, "ffmpeg"), false);
});

test("nothing is missing once all three are ready", () => {
  assert.deepEqual(missingTools({ ytdlp: tool, ffmpeg: tool, jsRuntime: "Deno" }), []);
});

test("a row shows the running install first, then ready, then its error", () => {
  assert.equal(stepState(false, true, true, "old error"), "installing");
  assert.equal(stepState(true, false, false, undefined), "ready");
  assert.equal(stepState(true, false, false, "stale error"), "ready");
  assert.equal(stepState(false, false, true, "checksum mismatch"), "error");
  assert.equal(stepState(false, false, true, undefined), "waiting");
  assert.equal(stepState(false, false, false, undefined), "missing");
});
