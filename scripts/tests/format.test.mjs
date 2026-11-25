import assert from "node:assert/strict";
import test from "node:test";

import { formatDate, itemNoun, siteName } from "../../.test-build/format.js";
import { describeEnqueue } from "../../.test-build/download-input.js";

test("extractor keys show as the site's own name", () => {
  assert.equal(siteName("Youtube"), "YouTube");
  assert.equal(siteName("YoutubeTab"), "YouTube");
  assert.equal(siteName("TwitchVod"), "Twitch");
  assert.equal(siteName("SoundcloudPlaylist"), "SoundCloud");
  assert.equal(siteName("Twitter"), "X");
  assert.equal(siteName("PeerTube"), "PeerTube");
  assert.equal(siteName("Generic"), null);
  assert.equal(siteName(null), null);
});

test("older dates use English month names, like Today and Yesterday", () => {
  const old = new Date(2026, 10, 23, 12).getTime();
  assert.equal(formatDate(old), "Nov 23, 2026");
  assert.match(formatDate(Date.now()), /^Today, /);
});

test("counts name audio downloads as audio files", () => {
  assert.equal(itemNoun("video", 1), "video");
  assert.equal(itemNoun("video", 3), "videos");
  assert.equal(itemNoun("audio", 1), "audio file");
  assert.equal(itemNoun("audio", 2), "audio files");
  const summary = { added: 2, skipped: 0, failed: 0, rowsAdded: 2, errors: [] };
  assert.equal(describeEnqueue(summary, ["audio file", "audio files"]), "Added 2 audio files to the queue.");
});
