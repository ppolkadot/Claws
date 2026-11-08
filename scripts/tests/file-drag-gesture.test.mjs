// Tests for src/ui/drag-gesture.ts, compiled first into .test-build/ (see package.json
// "test:motion"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --rootDir src --outDir .test-build src/ui/drag-gesture.ts
//   node --test scripts/tests/file-drag-gesture.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import { DRAG_THRESHOLD, idleGesture, move, outcomeMessage, press, release, revealsOnClick, settle } from "../../.test-build/ui/drag-gesture.js";

const PRIMARY = 1;

test("a primary press becomes a drag once, after the pointer passes the threshold", () => {
  let g = press(idleGesture, 0, 100, 100);
  assert.equal(g.phase, "pressed");
  let step = move(g, 100 + DRAG_THRESHOLD - 1, 100, PRIMARY);
  assert.equal(step.start, false, "a small wobble is still a click");
  g = step.gesture;
  step = move(g, 100 + 3, 100 + 4, PRIMARY); // distance 5
  assert.equal(step.start, true);
  g = step.gesture;
  assert.equal(g.phase, "dragging");
  // Further moves (if the page still sees any) never start a second native drag.
  assert.equal(move(g, 300, 300, PRIMARY).start, false);
});

test("secondary buttons and presses during a drag are ignored", () => {
  assert.equal(press(idleGesture, 2, 0, 0).phase, "idle");
  assert.equal(press(idleGesture, 1, 0, 0).phase, "idle");
  const dragging = move(press(idleGesture, 0, 0, 0), 50, 0, PRIMARY).gesture;
  assert.equal(press(dragging, 0, 10, 10), dragging, "a press cannot restart a drag still in progress");
});

test("touch and pen never start a native drag; their tap shows the file instead", () => {
  for (const type of ["touch", "pen"]) {
    const g = press(idleGesture, 0, 0, 0, type);
    assert.equal(g.phase, "idle", type);
    assert.equal(move(g, 80, 80, PRIMARY).start, false, `${type}: a swipe is not a drag`);
    assert.equal(revealsOnClick(g, 1), true, `${type}: the tap shows the file in its folder`);
    // Even straight after a mouse drag, whose ending click is otherwise ignored.
    const afterDrag = settle(move(press(idleGesture, 0, 0, 0, "mouse"), 50, 0, PRIMARY).gesture);
    assert.equal(revealsOnClick(press(afterDrag, 0, 0, 0, type), 1), true, `${type} after a mouse drag`);
  }
  const dragging = move(press(idleGesture, 0, 0, 0, "mouse"), 50, 0, PRIMARY).gesture;
  assert.equal(press(dragging, 0, 0, 0, "touch"), dragging, "a touch cannot reset a drag in progress");
});

test("a release the page missed ends the press without dragging", () => {
  const g = press(idleGesture, 0, 0, 0);
  const step = move(g, 50, 50, 0); // the button is already up
  assert.equal(step.start, false);
  assert.equal(step.gesture.phase, "idle");
  assert.equal(release(g).phase, "idle");
  assert.equal(release(idleGesture), idleGesture);
});

test("the native drag's own result ends it, since the OLE loop swallows pointerup", () => {
  const dragging = move(press(idleGesture, 0, 0, 0), 50, 0, PRIMARY).gesture;
  assert.equal(release(dragging).phase, "dragging", "a stray pointerup does not end it early");
  const settled = settle(dragging);
  assert.equal(settled.phase, "idle");
  // Ready for the next drag.
  assert.equal(move(press(settled, 0, 0, 0), 0, 9, PRIMARY).start, true);
});

test("Enter, Space and plain clicks show the file; the click that ends a drag does not", () => {
  assert.equal(revealsOnClick(idleGesture, 0), true, "keyboard activation");
  const pressed = press(idleGesture, 0, 0, 0);
  assert.equal(revealsOnClick(release(pressed), 1), true, "a click without movement");
  const dragging = move(pressed, 50, 0, PRIMARY).gesture;
  assert.equal(revealsOnClick(dragging, 0), false, "nothing while the drag runs");
  const settled = settle(dragging);
  assert.equal(revealsOnClick(settled, 1), false, "the mouse click that ended the drag");
  assert.equal(revealsOnClick(settled, 0), true, "keyboard still works after a drag");
  assert.equal(revealsOnClick(release(press(settled, 0, 0, 0)), 1), true, "a fresh click works again");
});

test("no outcome is reported as a copy; only a drop on Claws itself explains itself", () => {
  for (const outcome of ["dropped", "cancelled", "notStarted"]) assert.equal(outcomeMessage(outcome), null, outcome);
  const message = outcomeMessage("droppedOnSelf");
  assert.match(message, /outside Claws/);
  assert.doesNotMatch(message, /copied|imported|success/i);
});
