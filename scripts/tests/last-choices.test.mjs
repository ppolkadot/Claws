import assert from "node:assert/strict";
import { test } from "node:test";

import { LAST_CHOICES_KEY, applyLastChoices, loadLastChoices, parseLastChoices, saveLastChoices } from "../../.test-build/last-choices.js";

const saved = { mode: "audio", audioFormat: "flac", height: 1080, videoPreset: "original", fastSave: true };
const offer = { heights: [2160, 1080, 720], audioFormats: ["mp3", "m4a", "flac"], transcript: true };

function memory(initial = {}) {
  const data = new Map(Object.entries(initial));
  return { getItem: (key) => data.get(key) ?? null, setItem: (key, value) => void data.set(key, value), data };
}

test("choices are saved and read back", () => {
  const storage = memory();
  saveLastChoices(saved, storage);
  assert.deepEqual(loadLastChoices(storage), saved);
  assert.ok(storage.data.has(LAST_CHOICES_KEY));
});

test("nothing saved, or something this version did not write, reads as no choices", () => {
  assert.equal(loadLastChoices(memory()), null);
  for (const text of ["", "not json", "null", "[]", '{"mode":"video"}', JSON.stringify({ ...saved, mode: "karaoke" }), JSON.stringify({ ...saved, height: -4 }), JSON.stringify({ ...saved, fastSave: "yes" })]) {
    assert.equal(parseLastChoices(text), null, text);
  }
  assert.deepEqual(parseLastChoices(JSON.stringify({ ...saved, height: null })), { ...saved, height: null });
});

test("storage that throws or is missing never breaks the page", () => {
  const broken = {
    getItem() {
      throw new Error("blocked");
    },
    setItem() {
      throw new Error("full");
    },
  };
  assert.equal(loadLastChoices(broken), null);
  assert.doesNotThrow(() => saveLastChoices(saved, broken));
  assert.equal(loadLastChoices(null), null);
  assert.doesNotThrow(() => saveLastChoices(saved, null));
});

test("remembered choices apply as they are when the link offers them", () => {
  assert.deepEqual(applyLastChoices(saved, offer), saved);
});

test("a cap the source does not offer goes back to Best", () => {
  assert.equal(applyLastChoices({ ...saved, height: 1440 }, offer).height, null);
  assert.equal(applyLastChoices({ ...saved, height: 1080 }, { ...offer, heights: [] }).height, null);
  assert.equal(applyLastChoices({ ...saved, height: null }, offer).height, null);
});

test("a transcript the link cannot give goes back to video, and an unknown audio format to the first", () => {
  const transcript = { ...saved, mode: "transcript" };
  assert.equal(applyLastChoices(transcript, offer).mode, "transcript");
  assert.equal(applyLastChoices(transcript, { ...offer, transcript: false }).mode, "video");
  assert.equal(applyLastChoices({ ...saved, audioFormat: "ogg" }, offer).audioFormat, "mp3");
});
