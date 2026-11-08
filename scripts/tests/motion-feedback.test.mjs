import assert from "node:assert/strict";
import { test } from "node:test";
import { completionFeedback } from "../../.test-build/ui/motion.js";
import { paintJobStatus } from "../../.test-build/ui/job-status.js";

test("completion cancels superseded animation and respects reduced motion and hidden windows", () => {
  let reduced = false;
  globalThis.matchMedia = () => ({ matches: reduced });
  globalThis.document = { hidden: false };
  let started = 0, cancelled = 0;
  const el = { animate(_frames, options) {
    started++;
    assert.ok(options.duration <= 250);
    return { cancel() { cancelled++; }, onfinish: null };
  } };
  completionFeedback(el, true);
  assert.equal(started, 1);
  completionFeedback(el, false);
  assert.equal(cancelled, 1);
  assert.equal(started, 1);
  reduced = true;
  completionFeedback(el, true);
  assert.equal(started, 1);
  reduced = false;
  document.hidden = true;
  completionFeedback(el, true);
  assert.equal(started, 1);
});

/** Just enough of an element for a status line: text, prepend and animate. */
function statusElement() {
  const el = {
    nodes: [],
    animations: 0,
    cancelled: 0,
    set textContent(text) { this.nodes = [text]; },
    prepend(...nodes) { this.nodes.unshift(...nodes); },
    animate() { el.animations++; return { cancel() { el.cancelled++; }, onfinish: null }; },
  };
  return el;
}

const check = () => ({ check: true });
const checks = (el) => el.nodes.filter((n) => n && n.check).length;

test("repainting a completed row never adds a second check", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const el = statusElement();
  paintJobStatus(el, "Processing the file", false, false, check);
  assert.equal(checks(el), 0);
  paintJobStatus(el, "Saved to Videos", true, true, check);
  for (let i = 0; i < 5; i++) paintJobStatus(el, "Saved to Videos", true, false, check);
  assert.equal(checks(el), 1);
  assert.deepEqual(el.nodes.slice(1), [" ", "Saved to Videos"]);
  assert.equal(el.animations, 1, "only the change to completed moves; repaints do not");
});

test("leaving completed (retry, removal) cancels the movement and drops the check", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const el = statusElement();
  paintJobStatus(el, "Saved to Videos", true, true, check);
  paintJobStatus(el, "Waiting for a free slot", false, true, check);
  assert.equal(el.cancelled, 1);
  assert.equal(checks(el), 0);
});

test("with reduced motion the check is static", () => {
  globalThis.matchMedia = () => ({ matches: true });
  globalThis.document = { hidden: false };
  const el = statusElement();
  paintJobStatus(el, "Saved to Videos", true, true, check);
  assert.equal(checks(el), 1, "the static indicator stays");
  assert.equal(el.animations, 0);
});
