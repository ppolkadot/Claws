// Tests for src/transcript-options.ts, compiled first into .test-build/ (see package.json
// "test:logic"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/transcript-options.ts
//   node --test scripts/tests/transcript-options.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  TRANSCRIPT_FORMATS,
  captionChoices,
  captionKey,
  defaultCaption,
  keptCaption,
  languageName,
  parseCaptionKey,
  transcriptFileSuffix,
  transcriptOptions,
  transcriptText,
} from "../../.test-build/transcript-options.js";

const lang = (code, name, auto) => ({ code, name, auto });

const probe = (over = {}) => ({ kind: "video", subtitles: [], captions: [], ...over });

test("human and automatic captions are offered apart, even for the same language", () => {
  const choices = captionChoices(
    probe({ captions: [lang("en", "English", false), lang("de", "German", false), lang("de-orig", "German (Original)", true), lang("en", "English", true)] }),
  );
  assert.deepEqual(choices.human.map((c) => c.code), ["en", "de"]);
  assert.deepEqual(choices.automatic.map((c) => c.code), ["de-orig", "en"]);
  assert.equal(choices.available, true);
  assert.notEqual(captionKey(choices.human[0]), captionKey(choices.automatic[1]), "en and automatic en are two choices");
});

test("no captions is an explicit unavailable state, and playlists never offer transcripts", () => {
  assert.equal(captionChoices(probe()).available, false);
  assert.equal(captionChoices(probe({ captions: null, subtitles: [] })).available, false);
  const list = probe({ kind: "playlist", captions: [lang("en", "English", false)] });
  assert.equal(captionChoices(list).available, false);
});

test("links saved before caption lists existed fall back to their subtitle list", () => {
  const saved = probe({ captions: undefined, subtitles: [lang("en", "English", false), lang("fr", "French (auto)", true)] });
  const choices = captionChoices(saved);
  assert.deepEqual(choices.human.map((c) => c.name), ["English"]);
  assert.deepEqual(choices.automatic, [], "old automatic entries can be machine translations, so they are not offered");
  const onlyAuto = probe({ captions: undefined, subtitles: [lang("fr", "French (auto)", true)] });
  assert.equal(captionChoices(onlyAuto).available, false);
});

test("the default prefers human captions, then the original-language automatic track", () => {
  assert.equal(defaultCaption(captionChoices(probe({ captions: [lang("en", "English", true), lang("ja", "Japanese", false)] }))), "human:ja");
  assert.equal(defaultCaption(captionChoices(probe({ captions: [lang("en", "English", true), lang("ja-orig", "Japanese (Original)", true)] }))), "auto:ja-orig");
  assert.equal(defaultCaption(captionChoices(probe({ captions: [lang("en", "English", true)] }))), "auto:en");
  assert.equal(defaultCaption(captionChoices(probe())), null);
});

test("the default is the video's own language, not the alphabetically first track", () => {
  // The probe lists human tracks by code: ar, de, en, es for an English talk.
  const talk = captionChoices(
    probe({
      captions: [lang("ar", "Arabic", false), lang("de", "German", false), lang("en", "English", false), lang("es", "Spanish", false), lang("en-orig", "English (Original)", true), lang("en", "English", true)],
    }),
  );
  assert.equal(defaultCaption(talk), "human:en");
  assert.equal(defaultCaption(talk, ["de-DE", "de"]), "human:en", "the spoken language beats the user's");
  // Same base language: an English (US) human track for an en-orig video.
  const us = captionChoices(probe({ captions: [lang("de", "German", false), lang("en-US", "English (US)", false), lang("en-orig", "English (Original)", true)] }));
  assert.equal(defaultCaption(us), "human:en-US");
  // No original-language track: the user's language, then English, then the first.
  const plain = captionChoices(probe({ captions: [lang("ar", "Arabic", false), lang("en", "English", false), lang("fr", "French", false)] }));
  assert.equal(defaultCaption(plain, ["fr-CA"]), "human:fr");
  assert.equal(defaultCaption(plain, ["ja"]), "human:en");
  assert.equal(defaultCaption(captionChoices(probe({ captions: [lang("ar", "Arabic", false), lang("de", "German", false)] })), ["ja"]), "human:ar");
  // The original language has only automatic captions: the user's human track is still taken.
  const ja = captionChoices(probe({ captions: [lang("ar", "Arabic", false), lang("en", "English", false), lang("ja-orig", "Japanese (Original)", true)] }));
  assert.equal(defaultCaption(ja, ["en-GB"]), "human:en");
  assert.equal(keptCaption("human:gone", talk, []), "human:en");
});

test("a choice survives a refresh only while it is still offered", () => {
  const before = captionChoices(probe({ captions: [lang("en", "English", false), lang("fr", "French", true)] }));
  const after = captionChoices(probe({ captions: [lang("fr", "French", true)] }));
  assert.equal(keptCaption("auto:fr", after), "auto:fr");
  assert.equal(keptCaption("human:en", after), "auto:fr", "gone: back to the default");
  assert.equal(keptCaption(null, before), "human:en");
  assert.equal(keptCaption("human:en", captionChoices(probe())), null);
});

test("caption keys round-trip, including codes with hyphens and dots", () => {
  for (const c of [lang("zh-Hans", "Chinese", false), lang("en.x", "English", true), lang("en-orig", "English (Original)", true)]) {
    assert.deepEqual(parseCaptionKey(captionKey(c)), { code: c.code, auto: c.auto });
  }
  assert.equal(parseCaptionKey("nonsense"), null);
  assert.equal(parseCaptionKey("auto:"), null);
});

test("the options sent for a transcript carry one language and no media choices", () => {
  assert.deepEqual(transcriptOptions("auto:en-orig", "srt"), {
    mode: "transcript",
    maxHeight: null,
    videoPreset: "compatible",
    audioFormat: "mp3",
    subtitleLangs: ["en-orig"],
    autoSubs: true,
    embedSubs: false,
    fastSave: false,
    transcriptFormat: "srt",
  });
  assert.equal(transcriptOptions(null, "txt"), null);
});

test("formats and the queue's description name what is saved", () => {
  assert.deepEqual(TRANSCRIPT_FORMATS.map((f) => f.value), ["txt", "txtTimestamps", "srt", "vtt"]);
  const options = transcriptOptions("auto:en", "txtTimestamps");
  // The same words as the Library (transcript.rs `quality`): names, not codes.
  assert.equal(transcriptText(options), "Transcript · English (auto-generated) · TXT with timestamps");
  assert.equal(transcriptText(transcriptOptions("human:ja", "vtt")), "Transcript · Japanese · VTT");
  assert.equal(transcriptText(transcriptOptions("auto:en-orig", "txt")), "Transcript · English (original, auto-generated) · TXT");
  assert.equal(transcriptText(transcriptOptions("human:zh-Hans", "srt")), "Transcript · Chinese (Simplified) · SRT");
  assert.equal(transcriptText(transcriptOptions("human:xx-yy", "srt")), "Transcript · xx-yy · SRT", "an unknown code is shown as it is");
  // Saved before the format existed: plain text.
  assert.equal(transcriptText({ ...options, autoSubs: false, transcriptFormat: undefined }), "Transcript · English · TXT");
  assert.equal(languageName("fr-CA"), "French");
});

test("file names follow transcript.rs: every track and format has its own", () => {
  assert.equal(transcriptFileSuffix(transcriptOptions("human:en", "txt")), " [en].txt");
  assert.equal(transcriptFileSuffix(transcriptOptions("auto:en", "txtTimestamps")), " [en auto, timestamps].txt");
  assert.equal(transcriptFileSuffix(transcriptOptions("auto:en-orig", "srt")), " [en-orig auto].srt");
  const names = new Set();
  for (const key of ["human:en", "auto:en", "auto:en-orig"]) {
    for (const format of ["txt", "txtTimestamps", "srt", "vtt"]) names.add(transcriptFileSuffix(transcriptOptions(key, format)));
  }
  assert.equal(names.size, 12);
});
