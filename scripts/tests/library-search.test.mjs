import assert from "node:assert/strict";
import { test } from "node:test";
import { matchesLibrarySearch } from "../../.test-build/library-search.js";

const spotify = { title: "Believe (Official Audio)", uploader: "Cher Official", matchOrigin: { spotifyTitle: "Believe", spotifyArtists: ["Cher", "Björk"] } };
const plain = { title: "Some Video", uploader: "Channel" };

test("finds the saved YouTube title and uploader", () => {
  assert.equal(matchesLibrarySearch(spotify, "official audio"), true);
  assert.equal(matchesLibrarySearch(plain, "channel"), true);
});
test("finds the Spotify title and each Spotify artist", () => {
  const diff = { title: "Lyrics video 123", uploader: "xyz", matchOrigin: { spotifyTitle: "Believe", spotifyArtists: ["Cher", "Björk"] } };
  assert.equal(matchesLibrarySearch(diff, "believe"), true);
  assert.equal(matchesLibrarySearch(diff, "cher"), true);
  assert.equal(matchesLibrarySearch(diff, "björk"), true);
});
test("case and surrounding whitespace do not matter; blank matches all", () => {
  assert.equal(matchesLibrarySearch(spotify, "  CHER  "), true);
  assert.equal(matchesLibrarySearch(plain, ""), true);
  assert.equal(matchesLibrarySearch(plain, "   "), true);
});
test("records without provenance behave as before, and misses stay misses", () => {
  assert.equal(matchesLibrarySearch(plain, "cher"), false);
  assert.equal(matchesLibrarySearch({ ...plain, matchOrigin: null }, "cher"), false);
  assert.equal(matchesLibrarySearch({ title: "t", uploader: null, matchOrigin: { spotifyTitle: null, spotifyArtists: [] } }, "x"), false);
});
