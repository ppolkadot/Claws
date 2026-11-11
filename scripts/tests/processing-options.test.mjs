// Tests for src/processing-options.ts, compiled first into .test-build/ (see package.json
// "test:logic").
import assert from "node:assert/strict";
import { test } from "node:test";

import { embedsCoverArt, fastSaveHint, skippedByFastSave } from "../../.test-build/processing-options.js";

const choices = (over = {}) => ({ mode: "audio", audioFormat: "mp3", metadata: false, embeddedSubs: false, fastSave: false, ...over });

test("cover art is embedded only for MP3 and M4A audio", () => {
  for (const format of ["mp3", "m4a"]) assert.equal(embedsCoverArt("audio", format), true, format);
  for (const format of ["opus", "flac", "wav", "best"]) assert.equal(embedsCoverArt("audio", format), false, format);
  assert.equal(embedsCoverArt("video", "mp3"), false);
  assert.equal(embedsCoverArt("transcript", "mp3"), false);
});

test("the skipped passes match the Rust policy for every audio format", () => {
  for (const format of ["mp3", "m4a", "opus", "flac", "wav", "best"]) {
    for (const metadata of [false, true]) {
      const skipped = skippedByFastSave(choices({ audioFormat: format, metadata }));
      const cover = format === "mp3" || format === "m4a";
      assert.equal(skipped.length, (metadata ? 1 : 0) + (cover ? 1 : 0), `${format} metadata=${metadata}`);
      assert.equal(skipped.some((s) => s.includes("cover art")), cover, format);
      assert.equal(skipped.some((s) => s.includes("title, uploader")), metadata, format);
      assert.equal(skipped.some((s) => s.includes("subtitles")), false, format);
    }
  }
});

test("the hint names the cover-art tradeoff and what stays", () => {
  const on = fastSaveHint(choices({ fastSave: true }));
  assert.match(on, /Skips embedding cover art \(MP3 and M4A files are saved without it\)\./);
  assert.match(on, /conversion, merge or container repair/);
  const off = fastSaveHint(choices({ fastSave: false }));
  assert.match(off, /^Turn on to skip embedding cover art/);
  assert.match(off, /conversion, merging and any container repair/);
});

test("the hint lists every skipped pass, and nothing when there is none", () => {
  const all = fastSaveHint(choices({ mode: "video", audioFormat: "mp3", metadata: true, embeddedSubs: true, fastSave: true }));
  assert.match(all, /writing title, uploader, date and chapters into the file and embedding subtitles/);
  assert.equal(all.includes("cover art"), false, "video downloads have no cover art pass");
  const three = skippedByFastSave(choices({ metadata: true, embeddedSubs: true }));
  assert.equal(three.length, 3);
  assert.match(fastSaveHint(choices({ metadata: true, embeddedSubs: true, fastSave: true })), /, .* and embedding cover art/);
  for (const format of ["opus", "flac", "wav", "best"]) {
    assert.match(fastSaveHint(choices({ audioFormat: format })), /^Nothing optional to skip/, format);
  }
  assert.match(fastSaveHint(choices({ mode: "video" })), /^Nothing optional to skip/);
});

test("Fast Save names SponsorBlock among what it skips, only when it was chosen", () => {
  const base = { mode: "video", audioFormat: "mp3", metadata: false, embeddedSubs: false, fastSave: true };
  assert.deepEqual(skippedByFastSave({ ...base, sponsorBlock: true }), ["SponsorBlock (nothing is marked or removed)"]);
  assert.deepEqual(skippedByFastSave({ ...base, sponsorBlock: false }), []);
  assert.deepEqual(skippedByFastSave(base), []);
  assert.match(fastSaveHint({ ...base, sponsorBlock: true }), /^Skips SponsorBlock/);
});
