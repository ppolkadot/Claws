// Tests for src/resolution.ts, compiled first into .test-build/ (see package.json
// "test:logic"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/resolution.ts
//   node --test scripts/tests/resolution.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  VIDEO_PRESETS,
  capChoices,
  describeResolution,
  hasExactSizes,
  keptQuality,
  qualityChoices,
  resolutionClass,
  videoOptionsText,
} from "../../.test-build/resolution.js";

/** A probe's resolution entry, as probe.rs reports it. */
const res = (width, height, extra = {}) => ({
  shortSide: width == null ? height : Math.min(width, height),
  width,
  height,
  vcodec: null,
  dynamicRange: null,
  fps: null,
  ...extra,
});

/** A single video's lookup with these sizes, largest first (probe.rs orders them). */
const video = (...resolutions) => ({ heights: resolutions.map((r) => r.shortSide), resolutions });

const values = (choices) => choices.map((c) => c.value);
const labels = (choices) => choices.map((c) => c.label);

test("an 8K source offers every size it has, with no 2160p ceiling", () => {
  const probe = video(res(7680, 4320), res(5760, 3240), res(5120, 2880), res(3840, 2160), res(2560, 1440), res(1920, 1080), res(1280, 720), res(256, 144));
  const choices = qualityChoices(probe, false);
  assert.deepEqual(values(choices), [4320, 3240, 2880, 2160, 1440, 1080, 720, 144]);
  assert.deepEqual(labels(choices), [
    "4320p · 7680×4320 · 8K",
    "3240p · 5760×3240 · 6K",
    "2880p · 5120×2880 · 5K",
    "2160p · 3840×2160 · 4K",
    "1440p · 2560×1440",
    "1080p · 1920×1080",
    "720p · 1280×720",
    "144p · 256×144",
  ]);
  for (const choice of choices) assert.equal(choice.hint, `Up to ${choice.value}p`);
});

test("only the sizes the source offers appear: no 6K or 8K for a 4K source", () => {
  const choices = qualityChoices(video(res(3840, 2160), res(1920, 1080)), false);
  assert.deepEqual(values(choices), [2160, 1080]);
  assert.ok(!labels(choices).some((l) => /[5-8]K/.test(l)), labels(choices).join(", "));
  // Odd sizes are offered as they are, not rounded onto a ladder.
  assert.deepEqual(values(qualityChoices(video(res(1916, 1076), res(1280, 716)), false)), [1076, 716]);
});

test("class labels follow the dimensions: every class needs both sides", () => {
  assert.equal(resolutionClass(7680, 4320), "8K");
  assert.equal(resolutionClass(8192, 4320), "8K", "DCI 8K");
  assert.equal(resolutionClass(4320, 7680), "8K", "portrait counts its sides, not its orientation");
  assert.equal(resolutionClass(5760, 3240), "6K");
  assert.equal(resolutionClass(6144, 3160), "6K", "DCI 6K");
  assert.equal(resolutionClass(5760, 1080), null, "a panorama is not named by its width alone");
  assert.equal(resolutionClass(3240, 5760), "6K", "portrait 6K");
  assert.equal(resolutionClass(3840, 2160), "4K");
  assert.equal(resolutionClass(4096, 2160), "4K", "DCI 4K");
  // No class above what the picture reaches: 5120×2880 is 5K, not 6K.
  assert.equal(resolutionClass(5120, 2880), "5K");
  // A 3240p picture is not 8K, an ultrawide 7680×3200 is 6K, not 8K, a 3840×1600 is not 4K.
  assert.equal(resolutionClass(7680, 3200), "6K");
  assert.equal(resolutionClass(3840, 1600), null);
  assert.equal(resolutionClass(2560, 1440), null);
  // Without a width nothing is claimed.
  assert.equal(resolutionClass(null, 4320), null);
  assert.equal(resolutionClass(undefined, 2160), null);
});

test("portrait sizes are named by their shorter side and marked portrait", () => {
  const choices = qualityChoices(video(res(2160, 3840), res(1080, 1920), res(720, 1280)), false);
  assert.deepEqual(values(choices), [2160, 1080, 720], "the value is what yt-dlp's res caps");
  assert.deepEqual(labels(choices), ["2160p · 2160×3840 portrait · 4K", "1080p · 1080×1920 portrait", "720p · 720×1280 portrait"]);
  assert.equal(describeResolution(res(1080, 1080)), "1080p · 1080×1080", "square is not portrait");
});

test("a size given without its width shows the height alone", () => {
  assert.equal(describeResolution(res(null, 2160)), "2160p");
  assert.equal(describeResolution(res(null, 4320)), "4320p", "no 8K claim without a width");
  assert.deepEqual(labels(qualityChoices(video(res(3840, 2160), res(null, 1440)), false)), ["2160p · 3840×2160 · 4K", "1440p"]);
});

test("a link saved before resolutions existed offers its heights only as caps", () => {
  const choices = qualityChoices({ heights: [4320, 2160, 1080, 90] }, false);
  assert.equal(hasExactSizes({ heights: [4320] }), false);
  assert.deepEqual(values(choices), [4320, 2160, 1080]);
  assert.deepEqual(labels(choices), ["Up to 4320p", "Up to 2160p", "Up to 1080p"], "no exact size or class claimed from a height alone");
  // Saved portrait heights were the longer side (1080×1920 saved as 1920): never named "1920p".
  assert.deepEqual(labels(qualityChoices({ heights: [1920, 1280], resolutions: [] }, false)), ["Up to 1920p", "Up to 1280p"]);
  assert.deepEqual(qualityChoices({ heights: [] }, false), [], "a single video with no sizes offers only Best");
});

test("a refresh that no longer offers the chosen size goes back to Best", () => {
  // A portrait link saved before exact sizes: "Up to 1280p" was chosen from the old heights.
  const cached = qualityChoices({ heights: [1920, 1280, 640] }, false);
  assert.equal(keptQuality(1280, cached), 1280);
  const refreshed = qualityChoices(video(res(1080, 1920), res(720, 1280), res(360, 640)), false);
  assert.deepEqual(values(refreshed), [1080, 720, 360]);
  assert.equal(keptQuality(1280, refreshed), null, "a cap no chip shows is never kept");
  // A size still offered is kept, and Best stays Best.
  assert.equal(keptQuality(720, refreshed), 720);
  assert.equal(keptQuality(null, refreshed), null);
  // An exact size that changed between lookups also goes back to Best.
  assert.equal(keptQuality(1076, qualityChoices(video(res(1920, 1080)), false)), null);
  // Generic caps on a list stay stable across refreshes.
  assert.equal(keptQuality(2880, qualityChoices({ heights: [], resolutions: [] }, true)), 2880);
});

test("lists without per-item sizes offer generic caps, labelled as caps", () => {
  for (const choices of [capChoices(), qualityChoices({ heights: [], resolutions: [] }, true)]) {
    assert.deepEqual(values(choices), [4320, 2880, 2160, 1440, 1080, 720, 480, 360]);
    assert.equal(choices[0].label, "Up to 4320p");
    assert.ok(choices.every((c) => c.label === `Up to ${c.value}p`), labels(choices).join(", "));
    assert.ok(!labels(choices).some((l) => /K\b/.test(l)), "no class without a source");
  }
  // A playlist whose own lookup had sizes uses them.
  assert.deepEqual(values(qualityChoices(video(res(1920, 1080)), true)), [1080]);
});

test("the presets offered are Compatible and Original, described honestly", () => {
  assert.deepEqual(
    VIDEO_PRESETS.map((p) => p.value),
    ["compatible", "original"],
    "the editing preset is not offered before it exists",
  );
  const [compatible, original] = VIDEO_PRESETS;
  assert.equal(compatible.label, "Compatible");
  assert.match(compatible.hint, /Prefers H\.264 video and AAC audio in MP4/);
  assert.match(compatible.hint, /another codec may be used/, "never implies every MP4 is H.264");
  assert.equal(original.label, "Original");
  assert.match(original.hint, /without re-encoding/);
  assert.match(original.hint, /MKV/);
  assert.match(original.hint, /some editors cannot open every codec/);
  assert.ok(!/repair|fix|restor/i.test(original.hint + compatible.hint), "no promise that remuxing repairs a codec");
});

test("queue rows name the size cap and a non-default preset", () => {
  assert.equal(videoOptionsText(null, "compatible"), "Video, best quality");
  assert.equal(videoOptionsText(1080, "compatible"), "Video up to 1080p");
  assert.equal(videoOptionsText(1080, undefined), "Video up to 1080p", "jobs from before presets");
  assert.equal(videoOptionsText(4320, "original"), "Video up to 4320p · Original");
  assert.equal(videoOptionsText(null, "original"), "Video, best quality · Original");
});
