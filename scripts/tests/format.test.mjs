import assert from "node:assert/strict";
import test from "node:test";

import { clock, formatBytesOf, formatDate, itemNoun, siteName } from "../../.test-build/format.js";
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

test("a clock reading shows zero as 0:00 and long times with hours", () => {
  assert.equal(clock(0), "0:00");
  assert.equal(clock(0.4), "0:00");
  assert.equal(clock(42), "0:42");
  assert.equal(clock(63), "1:03");
  assert.equal(clock(3723), "1:02:03");
  assert.equal(clock(null), "");
  assert.equal(clock(-1), "");
  assert.equal(clock(Number.NaN), "");
});

test("a byte count of a total uses the unit the total reads best in", () => {
  assert.equal(formatBytesOf(2_791_728_742, 5_856_547_590), "2.6 of 5.5 GB");
  assert.equal(formatBytesOf(0, 5_856_547_590), "0.0 of 5.5 GB");
  assert.equal(formatBytesOf(512, 2048), "0.5 of 2.0 KB");
  assert.equal(formatBytesOf(100, 900), "100 of 900 B");
  assert.equal(formatBytesOf(5, 1_572_864), "0.0 of 1.5 MB");
});
