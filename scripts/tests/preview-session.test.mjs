// Tests for src/preview-session.ts, compiled by "test:logic" (package.json). The Download view
// saves what a single link showed when you leave it and takes it back on return; these cover the
// rules for what is kept, for how long, and what ends it.
import assert from "node:assert/strict";
import { test } from "node:test";

import { ESTIMATE_CAP } from "../../.test-build/download-estimates.js";
import { PreviewMemory, estimatesStillApply } from "../../.test-build/preview-session.js";

const MB = 1024 * 1024;
const avail = (bytes) => ({
  downloadBytes: bytes,
  confidence: "approximate",
  parts: [],
  container: "mp4",
  peakTempBytes: bytes * 2,
  requiredFreeBytes: bytes * 3,
  outputBytes: bytes,
  converted: false,
  unknownReason: null,
  state: "available",
  diagnostic: null,
});
const failed = () => ({ ...avail(1), downloadBytes: null, state: "failed", diagnostic: "offline" });
const unknown = () => ({ ...avail(1), downloadBytes: null, state: "unknown" });

const saved = (over = {}) => ({
  url: "https://example.com/v",
  typedUrl: "https://example.com/v",
  result: { kind: "video", url: "https://example.com/v", title: "T", entries: [] },
  choices: { mode: "audio", height: 720, videoPreset: "original", audioFormat: "flac", subs: ["en", "pt"], embedSubs: true, fastSave: true, caption: "en:human", transcriptFormat: "srt" },
  selected: [0, 2],
  freshness: "fresh",
  staleReason: "",
  notices: ["A subtitle you chose is no longer offered."],
  list: null,
  ...over,
});

test("the choices made on a link come back as they were left", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved());
  const back = memory.take();
  assert.ok(back, "something was saved");
  assert.equal(back.choices.audioFormat, "flac");
  assert.deepEqual(back.choices.subs, ["en", "pt"]);
  assert.deepEqual(back.selected, [0, 2]);
  assert.deepEqual(back.notices, ["A subtitle you chose is no longer offered."]);
});

test("what is saved is a copy of plain data: later changes to the original do not leak in", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  const original = saved();
  memory.save(original);
  original.choices.subs.push("fr");
  original.selected.length = 0;
  const back = memory.take();
  assert.deepEqual(back.choices.subs, ["en", "pt"]);
  assert.deepEqual(back.selected, [0, 2]);
});

test("a callback or other non-data value is never kept: the session is dropped, not serialized", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved({ onChange: () => undefined }));
  assert.equal(memory.take(), null);
  assert.equal(memory.active, false, "and nothing half-saved is left to revive");
});

test("taking the saved view back consumes it, so a later return cannot revive an old one", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved());
  assert.ok(memory.take());
  assert.equal(memory.take(), null);
});

test("a replacement link starts over: the old choices and estimates are gone", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved());
  memory.remember("k", avail(MB));
  memory.begin("https://example.com/other");
  assert.equal(memory.take(), null);
  assert.equal(memory.recall("k"), undefined);
  assert.equal(memory.size, 0);
});

test("a save for a link that is no longer the session's is ignored", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/other");
  memory.save(saved());
  assert.equal(memory.take(), null);
});

test("clearing (a batch replaced it, or the link was added to the queue) ends everything", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved());
  memory.remember("k", avail(MB));
  memory.clear();
  assert.equal(memory.active, false);
  assert.equal(memory.take(), null);
  assert.equal(memory.recall("k"), undefined);
  memory.remember("k", avail(MB)); // no session: nothing to keep it for
  assert.equal(memory.size, 0);
});

test("an incompatible refresh moves to a new generation and drops estimates of the old details", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  const before = memory.generation;
  memory.remember("k", avail(MB));
  memory.refreshed();
  assert.equal(memory.generation, before + 1);
  assert.equal(memory.recall("k"), undefined);
  assert.equal(memory.size, 0);
});

test("only completed estimates with a size are remembered: failed and unknown ones are asked again", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.remember("failed", failed());
  memory.remember("unknown", unknown());
  memory.remember("good", avail(MB));
  assert.equal(memory.recall("failed"), undefined);
  assert.equal(memory.recall("unknown"), undefined);
  assert.equal(memory.recall("good").downloadBytes, MB);
});

test("the estimate cache is bounded by the estimate cap, dropping the least recently used", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  for (let i = 0; i < ESTIMATE_CAP; i++) memory.remember(`k${i}`, avail(MB));
  assert.equal(memory.size, ESTIMATE_CAP);
  memory.recall("k0"); // used again: no longer the oldest
  memory.remember("extra", avail(MB));
  assert.equal(memory.size, ESTIMATE_CAP);
  assert.equal(memory.recall("k1"), undefined, "the least recently used went");
  assert.ok(memory.recall("k0"));
  assert.ok(memory.recall("extra"));
});

test("a saved list run keeps at most the estimate cap of items", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  const settled = Array.from({ length: ESTIMATE_CAP + 30 }, (_, i) => ({ item: { url: `u${i}` }, result: avail(MB) }));
  memory.save(saved({ list: { key: "k", asked: settled.map((s) => s.item.url), total: settled.length, requested: settled.length, done: settled.length, cancelled: false, notEstimated: 0, settled } }));
  assert.equal(memory.take().list.settled.length, ESTIMATE_CAP);
});

test("only a change of what sizes depend on invalidates estimates, not a new title or thumbnail", () => {
  const base = { heights: [1080, 720], resolutions: [{ width: 1920, height: 1080 }], entries: [{ url: "a", playlistIndex: 1, title: "A" }] };
  const same = { ...base, entries: [{ url: "a", playlistIndex: 1, title: "Renamed" }], thumbnail: "x" };
  assert.equal(estimatesStillApply(base, same), true);
  assert.equal(estimatesStillApply(base, { ...base, heights: [720] }), false);
  assert.equal(estimatesStillApply(base, { ...base, resolutions: undefined }), false);
  assert.equal(estimatesStillApply(base, { ...base, entries: [{ url: "b", playlistIndex: 1, title: "A" }] }), false);
  assert.equal(estimatesStillApply(base, { ...base, entries: [{ url: "a", playlistIndex: 2, title: "A" }] }), false);
  assert.equal(estimatesStillApply(null, base), false);
});

test("a refresh that changes the length of the video, or of one in the list, invalidates estimates", () => {
  const base = { heights: [1080], duration: 600, entries: [{ url: "a", playlistIndex: 1, duration: 60 }] };
  assert.equal(estimatesStillApply(base, { ...base, entries: [{ url: "a", playlistIndex: 1, duration: 60 }] }), true);
  assert.equal(estimatesStillApply(base, { ...base, duration: 700 }), false);
  assert.equal(estimatesStillApply(base, { ...base, entries: [{ url: "a", playlistIndex: 1, duration: 61 }] }), false);
  assert.equal(estimatesStillApply({ heights: [1080], entries: [] }, { heights: [1080], duration: null, entries: [] }), true, "no length is no length");
});

test("clearing for a link ends only that link's session", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/a");
  memory.begin("https://example.com/b"); // another link was looked up while a was being added
  memory.remember("k", avail(MB));
  memory.save(saved({ url: "https://example.com/b" }));
  memory.clearFor("https://example.com/a");
  assert.equal(memory.active, true, "b's session survived a's enqueue finishing");
  assert.ok(memory.recall("k"));
  assert.ok(memory.take(), "and so did its saved view");
  memory.clearFor("https://example.com/b");
  assert.equal(memory.active, false);
  memory.clearFor("https://example.com/b"); // nothing left: harmless
});

test("a saved view of a link that was then added is not brought back", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.save(saved()); // the view was left while the add was still on its way
  memory.clearFor("https://example.com/v"); // the add finished
  assert.equal(memory.take(), null);
});

test("a view left while its link is being added is not brought back, even to a return before the add ends", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.adding("https://example.com/v");
  memory.save(saved()); // left during the enqueue
  assert.equal(memory.take(), null, "returned before the add finished: the link is on its way, not shown as new");
  memory.clearFor("https://example.com/v"); // the add finished
  assert.equal(memory.active, false);
});

test("a failed add leaves the session usable", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.adding("https://example.com/v");
  memory.addFailed("https://example.com/v");
  memory.save(saved());
  assert.ok(memory.take(), "the view comes back, the link was not added");
});

test("a link being added does not hold back another link's view", () => {
  const memory = new PreviewMemory();
  memory.adding("https://example.com/a");
  memory.begin("https://example.com/b");
  memory.save(saved({ url: "https://example.com/b" }));
  assert.ok(memory.take());
});

test("rekey follows a lookup that settled on another address, and no other link's save", () => {
  const memory = new PreviewMemory();
  memory.begin("https://example.com/v");
  memory.rekey("https://example.com/canonical");
  memory.save(saved({ url: "https://example.com/v" }));
  assert.equal(memory.take(), null, "the old address is no longer this session's");
  memory.save(saved({ url: "https://example.com/canonical" }));
  assert.ok(memory.take());
  const none = new PreviewMemory();
  none.rekey("https://example.com/x"); // no session: nothing to do
  assert.equal(none.active, false);
});
