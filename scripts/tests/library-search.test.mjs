import assert from "node:assert/strict";
import { test } from "node:test";
import { LIBRARY_KINDS, LIBRARY_ORDERS, matchesLibraryKind, matchesLibrarySearch, sortLibrary } from "../../.test-build/library-search.js";

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


const entry = (id, title, fileSize, finishedAt, mode = "video") => ({ id, title, fileSize, finishedAt, mode });
const shelf = [
  entry("a", "beta song", 500, 30, "audio"),
  entry("b", "Alpha film", 9000, 10),
  entry("c", "alpha Film 2", null, 40, "transcript"),
  entry("d", "Gamma", 9000, 20),
];
const ids = (items) => items.map((i) => i.id).join("");

test("the kind filter shows everything, or only one kind", () => {
  assert.equal(ids(shelf.filter((i) => matchesLibraryKind(i, "all"))), "abcd");
  assert.equal(ids(shelf.filter((i) => matchesLibraryKind(i, "video"))), "bd");
  assert.equal(ids(shelf.filter((i) => matchesLibraryKind(i, "audio"))), "a");
  assert.equal(ids(shelf.filter((i) => matchesLibraryKind(i, "transcript"))), "c");
  assert.deepEqual(LIBRARY_KINDS.map((k) => k.value), ["all", "video", "audio", "transcript"]);
});

test("by date the newest comes first", () => {
  assert.equal(ids(sortLibrary(shelf, "date")), "cadb");
  assert.deepEqual(LIBRARY_ORDERS.map((o) => o.value), ["date", "title", "size"]);
});

test("by title the order ignores case and reads numbers as numbers", () => {
  assert.equal(ids(sortLibrary(shelf, "title")), "bcad");
  const numbered = [entry("x", "Part 10", 1, 1), entry("y", "Part 2", 1, 2)];
  assert.equal(ids(sortLibrary(numbered, "title")), "yx");
});

test("by size the largest comes first, equal sizes newest first, and unknown sizes last", () => {
  assert.equal(ids(sortLibrary(shelf, "size")), "dbac");
});

test("sorting returns a new list and never depends on the order it was given", () => {
  const before = ids(shelf);
  const reversed = [...shelf].reverse();
  for (const order of ["date", "title", "size"]) assert.equal(ids(sortLibrary(reversed, order)), ids(sortLibrary(shelf, order)), order);
  assert.equal(ids(shelf), before);
});
