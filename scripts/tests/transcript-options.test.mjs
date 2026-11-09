// Tests for src/transcript-options.ts, compiled first into .test-build/ (see package.json
// "test:logic"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/transcript-options.ts
//   node --test scripts/tests/transcript-options.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  MAX_SUBTITLE_LANGS,
  SUBTITLE_CHIPS_SHOWN,
  TRANSCRIPT_FORMATS,
  canSelectSubtitle,
  captionChoices,
  captionKey,
  defaultCaption,
  keptCaption,
  languageName,
  offeredSubtitles,
  parseCaptionKey,
  refreshSignature,
  refreshTrackChoices,
  subtitleLimitHint,
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

// ---- Subtitle limit, refresh signatures and refresh-time retention (W8) ----

test("the picker's limit is the backend's limit", () => {
  const rust = readFileSync(new URL("../../src-tauri/src/downloader.rs", import.meta.url), "utf8");
  const match = /pub const MAX_SUBTITLE_LANGS: usize = (\d+);/.exec(rust);
  assert.ok(match, "downloader.rs exports MAX_SUBTITLE_LANGS");
  assert.equal(MAX_SUBTITLE_LANGS, Number(match[1]));
  assert.equal(MAX_SUBTITLE_LANGS, 12);
  assert.ok(SUBTITLE_CHIPS_SHOWN >= MAX_SUBTITLE_LANGS, "every selectable language can be listed");
});

test("the picker stops at the limit but never blocks removing a choice", () => {
  const upTo = (n) => new Set(Array.from({ length: n }, (_, i) => `l${i}`));
  assert.equal(canSelectSubtitle(upTo(0), "x"), true);
  assert.equal(canSelectSubtitle(upTo(MAX_SUBTITLE_LANGS - 1), "x"), true, "the twelfth is allowed");
  assert.equal(canSelectSubtitle(upTo(MAX_SUBTITLE_LANGS), "x"), false, "the thirteenth is not");
  assert.equal(canSelectSubtitle(upTo(MAX_SUBTITLE_LANGS), "l3"), true, "a chosen language can always be unchosen");
  // The hint always states the limit and says when it has been reached.
  assert.match(subtitleLimitHint(0), /12/);
  assert.match(subtitleLimitHint(MAX_SUBTITLE_LANGS - 1), /12/);
  assert.doesNotMatch(subtitleLimitHint(MAX_SUBTITLE_LANGS - 1), /limit reached/i);
  assert.match(subtitleLimitHint(MAX_SUBTITLE_LANGS), /limit reached/i);
  assert.equal(offeredSubtitles(Array.from({ length: 30 }, (_, i) => i)).length, SUBTITLE_CHIPS_SHOWN);
});

const entry = (over = {}) => ({ url: "https://e/v1", playlistIndex: null, title: "One", uploader: "Chan", duration: 60, thumbnail: "https://t/1.jpg", ...over });
const lookup = (over = {}) => ({
  kind: "video", title: "T", thumbnail: "https://t/a.jpg", duration: 90, uploader: "U", site: "Youtube", heights: [1080], resolutions: [],
  subtitles: [lang("en", "English", false), lang("fr", "French (auto)", true)],
  captions: [lang("en", "English", false), lang("fr", "French", true)], entries: [], ...over,
});

test("a refresh redraws when anything the page shows changes, and only then", () => {
  const base = refreshSignature(lookup());
  assert.equal(refreshSignature(lookup()), base, "an equal lookup is the same signature");
  const changes = {
    "subtitle display name": lookup({ subtitles: [lang("en", "English (United States)", false), lang("fr", "French (auto)", true)] }),
    "subtitle kind": lookup({ subtitles: [lang("en", "English", true), lang("fr", "French (auto)", true)] }),
    "subtitle code": lookup({ subtitles: [lang("en", "English", false), lang("de", "French (auto)", true)] }),
    "caption display name": lookup({ captions: [lang("en", "English", false), lang("fr", "Francais", true)] }),
    "caption kind": lookup({ captions: [lang("en", "English", false), lang("fr", "French", false)] }),
    "captions known to be none": lookup({ captions: null }),
    site: lookup({ site: "Vimeo" }),
    uploader: lookup({ uploader: "Other" }),
    title: lookup({ title: "T2" }),
  };
  for (const [what, changed] of Object.entries(changes)) assert.notEqual(refreshSignature(changed), base, what);

  const playlistBase = refreshSignature(lookup({ kind: "playlist", entries: [entry()] }));
  const playlistChanges = {
    "entry uploader": entry({ uploader: "Else" }),
    "entry duration": entry({ duration: 61 }),
    "entry thumbnail": entry({ thumbnail: null }),
    "entry position": entry({ playlistIndex: 3 }),
    "entry title": entry({ title: "Uno" }),
    "entry link": entry({ url: "https://e/v2" }),
  };
  for (const [what, e] of Object.entries(playlistChanges)) assert.notEqual(refreshSignature(lookup({ kind: "playlist", entries: [e] })), playlistBase, what);
});

const choicesOf = (over = {}) => ({
  mode: "video", subs: new Set(["en", "fr"]), caption: "human:en",
  height: 720, videoPreset: "original", audioFormat: "flac", embedSubs: true, fastSave: true, transcriptFormat: "srt", ...over,
});
const tracks = (list) => ({ kind: "video", subtitles: list, captions: list });

test("a track whose name alone changed stays chosen, with no notice and every other choice untouched", () => {
  const before = tracks([lang("en", "English", false), lang("fr", "French (auto)", true)]);
  const after = tracks([lang("en", "English (US)", false), lang("fr", "Francais (auto)", true)]);
  const { choices, notices } = refreshTrackChoices(choicesOf(), before, after);
  assert.deepEqual([...choices.subs], ["en", "fr"]);
  assert.equal(choices.caption, "human:en");
  assert.deepEqual(notices, []);
  const { subs, caption, mode, ...others } = choices;
  assert.deepEqual(others, { height: 720, videoPreset: "original", audioFormat: "flac", embedSubs: true, fastSave: true, transcriptFormat: "srt" });
  assert.equal(mode, "video");
});

test("a track that is now a different kind is deselected and named, not silently swapped", () => {
  const before = tracks([lang("en", "English", false), lang("fr", "French (auto)", true)]);
  // English lost its human track (now automatic only); French is unchanged.
  const after = tracks([lang("en", "English (auto)", true), lang("fr", "French (auto)", true)]);
  const { choices, notices } = refreshTrackChoices(choicesOf(), before, after);
  assert.deepEqual([...choices.subs], ["fr"], "only the track that is still what was chosen stays");
  assert.equal(notices.length, 1);
  assert.match(notices[0], /English/);
  assert.doesNotMatch(notices[0], /French/);
  // Unrelated choices are exactly as they were.
  assert.equal(choices.height, 720);
  assert.equal(choices.audioFormat, "flac");
  assert.equal(choices.transcriptFormat, "srt");
  assert.equal(choices.videoPreset, "original");
});

test("a track the refresh dropped, or moved past the listed chips, is not kept hidden", () => {
  const many = Array.from({ length: 20 }, (_, i) => lang(`l${i}`, `Language ${i}`, false));
  const before = tracks(many);
  const reordered = tracks([...many.slice(10), ...many.slice(0, 10)]);
  const { choices, notices } = refreshTrackChoices(choicesOf({ subs: new Set(["l8", "l12"]) }), before, reordered);
  assert.deepEqual([...choices.subs], ["l12"], "l8 is no longer among the listed chips, so it cannot stay chosen out of sight");
  assert.match(notices[0], /Language 8/);
  assert.equal(refreshTrackChoices(choicesOf({ subs: new Set(["l1"]) }), before, tracks([])).choices.subs.size, 0);
});

test("a transcript's caption is kept while offered, replaced with a notice when its kind is gone", () => {
  const before = tracks([lang("es", "Spanish", false), lang("en", "English", false)]);
  const sameKind = tracks([lang("es", "Espanol", false), lang("en", "English", false)]);
  const kept = refreshTrackChoices(choicesOf({ mode: "transcript", caption: "human:es", subs: new Set() }), before, sameKind);
  assert.equal(kept.choices.caption, "human:es");
  assert.deepEqual(kept.notices, []);
  assert.equal(kept.choices.mode, "transcript");

  const noMoreHuman = { kind: "video", subtitles: [], captions: [lang("es", "Spanish", true), lang("en", "English", false)] };
  const swapped = refreshTrackChoices(choicesOf({ mode: "transcript", caption: "human:es", subs: new Set() }), before, noMoreHuman);
  assert.notEqual(swapped.choices.caption, "human:es");
  assert.equal(swapped.notices.length, 1);
  assert.match(swapped.notices[0], /Spanish/);
  // Outside Transcript the stored caption is only a default: no notice for a choice not in use.
  const quiet = refreshTrackChoices(choicesOf({ mode: "video", caption: "human:es", subs: new Set() }), before, noMoreHuman);
  assert.deepEqual(quiet.notices, []);
});

test("a transcript with no captions left falls back to video and says why", () => {
  const before = tracks([lang("en", "English", false)]);
  const none = { kind: "video", subtitles: [], captions: [] };
  const { choices, notices } = refreshTrackChoices(choicesOf({ mode: "transcript", caption: "human:en", subs: new Set(), height: 480 }), before, none);
  assert.equal(choices.mode, "video");
  assert.equal(choices.height, 480);
  assert.equal(notices.length, 1);
  assert.match(notices[0], /captions/i);
});
