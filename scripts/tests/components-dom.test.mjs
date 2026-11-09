// Behavior tests for confirmSheet and chipRadio in src/ui/components.ts, run against the
// fake DOM in scripts/tests/fake-dom.mjs (compiled by "test:motion" in package.json).
import assert from "node:assert/strict";
import { test } from "node:test";

import { chipRadio, confirmSheet } from "../../.test-build/ui/components.js";
import { subtitleChips } from "../../.test-build/ui/subtitle-chips.js";
import { MAX_SUBTITLE_LANGS } from "../../.test-build/transcript-options.js";
import { FakeEvent, findAll, installFakeDom, keydown } from "./fake-dom.mjs";

const tick = () => new Promise((resolve) => setImmediate(resolve));

/** Opens a confirmation and reports whether (and with what) it has resolved. */
function openSheet(options = {}) {
  const document = installFakeDom(options);
  const state = { settled: false, value: undefined, settleCount: 0 };
  const promise = confirmSheet("Delete?", "This cannot be undone.", [{ label: "Delete", value: "delete", variant: "danger" }]);
  promise.then((value) => {
    state.settled = true;
    state.value = value;
    state.settleCount += 1;
  });
  const dialog = document.body.children.find((c) => c.tagName === "DIALOG");
  const buttons = findAll(dialog, (n) => n.tagName === "BUTTON");
  const byLabel = (label) => buttons.find((b) => b.textContent === label);
  return { document, dialog, state, deleteButton: byLabel("Delete"), cancelButton: byLabel("Cancel") };
}

const escape = (dialog) => dialog.dispatchEvent(new FakeEvent("cancel", { bubbles: false }));

test("Cancel then Delete: the first decision wins and the dismissal is never destructive", async () => {
  const { dialog, state, deleteButton, cancelButton } = openSheet();
  cancelButton.click();
  assert.equal(dialog.animations.length, 1, "the departure starts once");
  deleteButton.click();
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, null);
  assert.equal(state.settleCount, 1);
});

test("Delete then Cancel: the confirmation stands and Cancel does nothing", async () => {
  const { dialog, state, deleteButton, cancelButton } = openSheet();
  deleteButton.click();
  cancelButton.click();
  assert.equal(dialog.animations.length, 1, "a later click does not start a second departure");
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, "delete");
  assert.equal(state.settleCount, 1);
});

test("both actions are disabled as soon as a decision is made", () => {
  const { deleteButton, cancelButton } = openSheet();
  assert.equal(deleteButton.disabled, false);
  cancelButton.click();
  assert.equal(deleteButton.disabled, true);
  assert.equal(cancelButton.disabled, true);
});

test("repeated Escape starts one departure and settles once with null", async () => {
  const { dialog, state } = openSheet();
  escape(dialog);
  escape(dialog);
  escape(dialog);
  assert.equal(dialog.animations.length, 1);
  dialog.animations[0].finish();
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, null);
  assert.equal(state.settleCount, 1);
});

test("Escape during a Delete departure does not cancel the confirmed choice", async () => {
  const { dialog, state, deleteButton } = openSheet();
  deleteButton.click();
  escape(dialog);
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, "delete");
});

test("Delete during an Escape departure cannot turn it destructive", async () => {
  const { dialog, state, deleteButton } = openSheet();
  escape(dialog);
  deleteButton.click();
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, null);
});

test("reduced motion: a decision closes immediately and settles once", async () => {
  const { dialog, state, deleteButton, cancelButton } = openSheet({ reducedMotion: true });
  deleteButton.click();
  cancelButton.click();
  escape(dialog);
  await tick();
  assert.equal(dialog.animations.length, 0);
  assert.equal(dialog.open, false);
  assert.equal(state.value, "delete");
  assert.equal(state.settleCount, 1);
});

test("hidden window: Cancel closes immediately and a later Delete is ignored", async () => {
  const { dialog, state, deleteButton, cancelButton } = openSheet({ hidden: true });
  cancelButton.click();
  deleteButton.click();
  await tick();
  assert.equal(dialog.animations.length, 0);
  assert.equal(dialog.open, false);
  assert.equal(state.value, null);
  assert.equal(state.settleCount, 1);
});

test("a departure animation that never finishes or cancels still closes the sheet", async () => {
  const { document, dialog, state, deleteButton } = openSheet({ fakeTimers: true });
  deleteButton.click();
  assert.equal(dialog.animations.length, 1);
  await tick();
  assert.equal(state.settled, false, "still departing");
  document.runTimers();
  await tick();
  assert.equal(state.settled, true, "the fallback must not leave the sheet stuck open");
  assert.equal(dialog.open, false);
  assert.equal(state.value, "delete");
  assert.equal(state.settleCount, 1);
});

test("a normal finish settles once and leaves no fallback pending", async () => {
  const { document, dialog, state, cancelButton } = openSheet({ fakeTimers: true });
  cancelButton.click();
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.settleCount, 1);
  assert.equal(document.timers.size, 0, "the fallback timer is cleared once the sheet closes");
  document.runTimers();
  await tick();
  assert.equal(state.settleCount, 1);
});

test("a cancelled departure animation still closes the sheet with the first decision", async () => {
  const { dialog, state, deleteButton } = openSheet();
  deleteButton.click();
  dialog.animations[0].cancel();
  await tick();
  assert.equal(state.settled, true, "the sheet must not stay stuck open");
  assert.equal(dialog.open, false);
  assert.equal(state.value, "delete");
  assert.equal(state.settleCount, 1);
});

test("finishing after cancelling does not settle twice", async () => {
  const { dialog, state, cancelButton } = openSheet();
  cancelButton.click();
  dialog.animations[0].cancel();
  dialog.animations[0].finish();
  await tick();
  assert.equal(state.value, null);
  assert.equal(state.settleCount, 1);
});

// ---- chipRadio -------------------------------------------------------------------------

const options = [
  { value: "a", label: "A" },
  { value: "b", label: "B" },
  { value: "c", label: "C" },
];

function openRadio(initial, list = options) {
  const document = installFakeDom();
  const changes = [];
  const group = chipRadio("Choice", list, initial, (value) => changes.push(value));
  const chips = group.children;
  const checked = () => chips.map((c) => c.getAttribute("aria-checked"));
  const tabs = () => chips.map((c) => c.tabIndex);
  const press = (key, from) => {
    const event = keydown(key, { target: from });
    from.dispatchEvent(event);
    return event;
  };
  return { document, group, chips, changes, checked, tabs, press };
}

test("chipRadio has one Tab stop, on the checked chip", () => {
  const { tabs, checked } = openRadio("b");
  assert.deepEqual(checked(), ["false", "true", "false"]);
  assert.deepEqual(tabs(), [-1, 0, -1]);
});

test("an unknown initial value leaves nothing checked but the group reachable", () => {
  const { tabs, checked } = openRadio("zzz");
  assert.deepEqual(checked(), ["false", "false", "false"]);
  assert.deepEqual(tabs(), [0, -1, -1]);
});

test("mouse click checks, moves the Tab stop and fires onChange once", () => {
  const { chips, tabs, checked, changes } = openRadio("a");
  chips[2].click();
  assert.deepEqual(changes, ["c"]);
  assert.deepEqual(checked(), ["false", "false", "true"]);
  assert.deepEqual(tabs(), [-1, -1, 0]);
});

test("ArrowRight and ArrowDown move forward and wrap, with focus and checked together", () => {
  const { document, chips, tabs, checked, changes, press } = openRadio("b");
  const event = press("ArrowRight", chips[1]);
  assert.equal(event.defaultPrevented, true);
  assert.equal(document.activeElement, chips[2]);
  assert.deepEqual(checked(), ["false", "false", "true"]);
  assert.deepEqual(tabs(), [-1, -1, 0]);
  press("ArrowDown", chips[2]);
  assert.equal(document.activeElement, chips[0], "wraps to the first");
  assert.deepEqual(checked(), ["true", "false", "false"]);
  assert.deepEqual(changes, ["c", "a"], "one onChange per key press");
});

test("ArrowLeft and ArrowUp move back and wrap", () => {
  const { document, chips, checked, changes, press } = openRadio("a");
  press("ArrowLeft", chips[0]);
  assert.equal(document.activeElement, chips[2], "wraps to the last");
  assert.deepEqual(checked(), ["false", "false", "true"]);
  press("ArrowUp", chips[2]);
  assert.equal(document.activeElement, chips[1]);
  assert.deepEqual(changes, ["c", "b"]);
});

test("Home and End jump to the ends", () => {
  const { document, chips, checked, changes, press } = openRadio("b");
  press("End", chips[1]);
  assert.equal(document.activeElement, chips[2]);
  press("Home", chips[2]);
  assert.equal(document.activeElement, chips[0]);
  assert.deepEqual(checked(), ["true", "false", "false"]);
  assert.deepEqual(changes, ["c", "a"]);
});

test("arrows with nothing checked: from the Tab stop they step on, from outside they enter at the ends", () => {
  const stepping = openRadio("zzz");
  stepping.press("ArrowRight", stepping.chips[0]);
  assert.deepEqual(stepping.changes, ["b"]);
  const back = openRadio("zzz");
  back.press("ArrowLeft", back.chips[0]);
  assert.deepEqual(back.changes, ["c"], "wraps from the first chip");
  const entering = openRadio("zzz");
  entering.press("ArrowRight", entering.group);
  assert.deepEqual(entering.changes, ["a"]);
  const enteringBack = openRadio("zzz");
  enteringBack.press("ArrowLeft", enteringBack.group);
  assert.deepEqual(enteringBack.changes, ["c"]);
});

test("other keys and modified arrows are left alone", () => {
  const { chips, changes, press } = openRadio("a");
  assert.equal(press("Tab", chips[0]).defaultPrevented, false);
  assert.equal(press("a", chips[0]).defaultPrevented, false);
  assert.equal(press("ArrowRight", chips[0]).defaultPrevented, true);
  const alt = keydown("ArrowRight", { altKey: true, target: chips[1] });
  chips[1].dispatchEvent(alt);
  assert.equal(alt.defaultPrevented, false);
  assert.deepEqual(changes, ["b"]);
});

test("a single chip: arrows stay put and do not fire onChange", () => {
  const { chips, changes, press } = openRadio("a", [options[0]]);
  press("ArrowRight", chips[0]);
  press("End", chips[0]);
  assert.deepEqual(changes, []);
});

test("an empty list builds an empty group and ignores keys without dividing by zero", () => {
  const { group, changes } = openRadio("a", []);
  assert.equal(group.children.length, 0);
  assert.doesNotThrow(() => group.dispatchEvent(keydown("ArrowRight", { target: group })));
  assert.doesNotThrow(() => group.dispatchEvent(keydown("Home", { target: group })));
  assert.deepEqual(changes, []);
});

// ---- The subtitle picker's limit (src/ui/subtitle-chips.ts) -----------------------------

/** A picker over `count` tracks with `chosen` of them already chosen, and what it reported. */
function openPicker(count, chosen) {
  const document = installFakeDom();
  const tracks = Array.from({ length: count }, (_, i) => ({ code: `l${i}`, name: `Lang ${i}` }));
  const selected = new Set(tracks.slice(0, chosen).map((t) => t.code));
  const changes = [];
  const picker = subtitleChips(tracks, selected, (now) => changes.push([...now]));
  document.body.appendChild(picker);
  const chips = findAll(picker, (n) => n.className === "chip");
  return { picker, chips, selected, changes };
}

test("at the limit an unchosen chip is unavailable and a click on it chooses nothing", () => {
  const { chips, selected, changes } = openPicker(MAX_SUBTITLE_LANGS + 2, MAX_SUBTITLE_LANGS);
  const chosen = chips.slice(0, MAX_SUBTITLE_LANGS);
  const unchosen = chips.slice(MAX_SUBTITLE_LANGS);
  assert.ok(unchosen.every((c) => c.getAttribute("aria-disabled") === "true"));
  assert.ok(chosen.every((c) => c.getAttribute("aria-disabled") === null), "chosen chips stay usable");
  for (const chip of unchosen) chip.click();
  assert.equal(selected.size, MAX_SUBTITLE_LANGS);
  assert.deepEqual(changes, [], "the picker is not even told");
  assert.ok(unchosen.every((c) => c.getAttribute("aria-checked") === "false"));
});

test("at the limit a chosen chip can still be cleared", () => {
  const { chips, selected, changes } = openPicker(MAX_SUBTITLE_LANGS + 2, MAX_SUBTITLE_LANGS);
  chips[0].click();
  assert.equal(selected.size, MAX_SUBTITLE_LANGS - 1);
  assert.equal(chips[0].getAttribute("aria-checked"), "false");
  assert.equal(changes.length, 1);
});

test("below the limit every chip is available and a click chooses it", () => {
  const { chips, selected, changes } = openPicker(MAX_SUBTITLE_LANGS + 2, MAX_SUBTITLE_LANGS - 1);
  assert.ok(chips.every((c) => c.getAttribute("aria-disabled") === null));
  chips[MAX_SUBTITLE_LANGS].click();
  assert.equal(selected.size, MAX_SUBTITLE_LANGS);
  assert.equal(chips[MAX_SUBTITLE_LANGS].getAttribute("aria-checked"), "true");
  assert.equal(changes.length, 1);
});

test("the picker lists no more than the chips it offers", () => {
  const { chips } = openPicker(40, 0);
  assert.equal(chips.length, 14);
});
