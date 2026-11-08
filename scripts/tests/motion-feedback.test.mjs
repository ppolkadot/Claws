import assert from "node:assert/strict";
import { test } from "node:test";
import { completionFeedback } from "../../.test-build/ui/motion.js";

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
