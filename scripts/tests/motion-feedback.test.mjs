import assert from "node:assert/strict";
import { test } from "node:test";
import { cancelMotion, collapseOut, completionFeedback, expandIn, pop, settleIn } from "../../.test-build/ui/motion.js";
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

/** Just enough of a row for expandIn and collapseOut. */
function rowElement() {
  const el = {
    style: {},
    removed: false,
    running: [],
    getBoundingClientRect: () => ({ height: 62 }),
    remove() { el.removed = true; },
    animate(frames, options) {
      const animation = { frames, options, cancelled: false, onfinish: null, cancel() { this.cancelled = true; } };
      el.running.push(animation);
      return animation;
    },
  };
  return el;
}

test("a row's entry returns its animation, and is skipped with reduced motion or a hidden window", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const el = rowElement();
  assert.ok(expandIn(el));
  globalThis.matchMedia = () => ({ matches: true });
  assert.equal(expandIn(rowElement()), null);
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: true };
  assert.equal(expandIn(rowElement()), null);
});

test("removing a row that is still arriving cancels the arrival instead of stacking a second animation", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const el = rowElement();
  const entry = expandIn(el);
  collapseOut(el);
  assert.equal(entry.cancelled, true);
  assert.equal(el.running.length, 2);
  el.running[1].onfinish();
  assert.equal(el.removed, true);
});

test("a hidden window removes a row at once instead of animating it out", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: true };
  const el = rowElement();
  collapseOut(el);
  assert.equal(el.removed, true);
  assert.equal(el.running.length, 0);
});

test("view disposal cancels the arrival and completion movements it knows about", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const row = rowElement();
  const status = rowElement();
  const entry = expandIn(row);
  completionFeedback(status, true);
  cancelMotion(row, status);
  assert.equal(entry.cancelled, true);
  assert.equal(status.running[0].cancelled, true);
  cancelMotion(row, status); // nothing left: no error
});

test("a row's departure returns its animation so leaving the view can cancel it; none in a hidden window", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const el = rowElement();
  const leaving = collapseOut(el);
  assert.ok(leaving, "the collapse is returned");
  cancelMotion(el);
  assert.equal(leaving.cancelled, true, "cancelMotion reaches a row that is collapsing out");
  globalThis.document = { hidden: true };
  assert.equal(collapseOut(rowElement()), null);
});

test("settleIn and pop start nothing with reduced motion or in a hidden window", () => {
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.document = { hidden: false };
  const shown = rowElement();
  assert.ok(settleIn(shown));
  pop(shown);
  assert.equal(shown.running.length, 2);
  for (const [reduced, hidden] of [[true, false], [false, true]]) {
    globalThis.matchMedia = () => ({ matches: reduced });
    globalThis.document = { hidden };
    const el = rowElement();
    assert.equal(settleIn(el), null);
    pop(el);
    assert.equal(el.running.length, 0, `reduced=${reduced} hidden=${hidden}`);
  }
});
