// Tests for src/kept-files.ts, compiled first into .test-build/ (see package.json "test:logic").
import assert from "node:assert/strict";
import { test } from "node:test";

import { keptCount, keptKey, keptLabel, keptClearWarning, keptOutcome, keptRemovalWarning, repairNote } from "../../.test-build/kept-files.js";

const FOLDER = "D:/Out/.claws-work/abc";
const entry = (sources = ["Clip.en.srt"], path = FOLDER) => ({ keptFiles: { workspace: { path }, sources } });

test("an entry with kept files is marked, one without is not", () => {
  assert.equal(keptCount(undefined), 0);
  assert.equal(keptCount({}), 0);
  assert.equal(keptCount({ keptFiles: null }), 0);
  assert.equal(keptLabel({}), null);
  assert.equal(keptLabel({ keptFiles: null }), null);
  assert.equal(keptLabel(entry()), "1 subtitle file kept separately");
  assert.equal(keptLabel(entry(["a.srt", "b.srt"])), "2 subtitle files kept separately");
});

test("the key changes with the kept files and is empty without them", () => {
  assert.equal(keptKey({}), "");
  assert.equal(keptKey(undefined), "");
  assert.notEqual(keptKey(entry()), "");
  assert.notEqual(keptKey(entry()), keptKey(entry(["a.srt", "b.srt"])));
  assert.notEqual(keptKey(entry()), keptKey(entry(["Clip.en.srt"], "E:/Other/.claws-work/abc")));
});

test("a shown file is told apart from an offline, missing or unproven folder", () => {
  const shown = keptOutcome({ status: "shown", folder: FOLDER, files: 1 });
  assert.equal(shown.tone, "info");
  assert.match(shown.text, /Showing 1 subtitle file/);
  const offline = keptOutcome({ status: "offline", folder: FOLDER, files: 0 });
  const missing = keptOutcome({ status: "missing", folder: FOLDER, files: 0 });
  const unverified = keptOutcome({ status: "unverified", folder: FOLDER, files: 0 });
  for (const outcome of [offline, missing, unverified]) {
    assert.equal(outcome.tone, "error");
    assert.ok(outcome.text.includes(FOLDER), outcome.text);
  }
  assert.match(offline.text, /cannot be reached/);
  assert.match(missing.text, /moved or deleted/);
  assert.match(unverified.text, /cannot confirm/);
  assert.equal(new Set([shown.text, offline.text, missing.text, unverified.text]).size, 4);
});

test("removal warns that kept files stay, naming the folder, only when there are some", () => {
  assert.equal(keptRemovalWarning({}), null);
  assert.equal(keptRemovalWarning({ keptFiles: null }), null);
  const warning = keptRemovalWarning(entry(["a.srt", "b.srt"]));
  assert.ok(warning.includes("2 subtitle files"), warning);
  assert.ok(warning.includes(`will stay in ${FOLDER}`), warning);
  assert.match(warning, /whichever option you choose/);
});

test("clearing the Library warns about every entry whose kept files would lose their record", () => {
  assert.equal(keptClearWarning([]), null);
  assert.equal(keptClearWarning([{}, { keptFiles: null }, undefined]), null);
  const one = keptClearWarning([{}, entry()]);
  assert.match(one, /1 download has kept subtitle files/);
  assert.match(one, /stay in their work folders/);
  assert.match(one, /can no longer show/);
  const two = keptClearWarning([entry(), entry(["a.srt", "b.srt"], "E:/Other/.claws-work/def"), {}]);
  assert.match(two, /2 downloads have kept subtitle files/);
});

test("the Library notes folders that could not be checked, and nothing otherwise", () => {
  assert.equal(repairNote(null), null);
  assert.equal(repairNote(undefined), null);
  assert.equal(repairNote({ attached: 2, inaccessible: [], ignored: 1 }), null);
  const one = repairNote({ attached: 0, inaccessible: ["E:/Videos"], ignored: 0 });
  assert.match(one, /this folder/);
  assert.ok(one.includes("E:/Videos"), one);
  const many = repairNote({ attached: 0, inaccessible: ["A:/", "B:/", "C:/", "D:/", "E:/"], ignored: 0 });
  assert.match(many, /these folders/);
  assert.match(many, /and 2 more/);
});
