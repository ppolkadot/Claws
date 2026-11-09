// Tests for src/library-cache.ts, compiled by "test:logic": the Library reuses a tile while
// nothing it shows or acts on changed, and rebuilds it once when something did. The tiles here are
// elements of the fake DOM, so focus staying on a reused tile is checked on a real node.
import assert from "node:assert/strict";
import { test } from "node:test";

import { TileCache, reconcileChildren, tileSignature } from "../../.test-build/library-cache.js";
import { installFakeDom } from "./fake-dom.mjs";

const item = (over = {}) => ({
  id: "a",
  title: "Title",
  url: "https://example.com/v",
  uploader: "Uploader",
  duration: 61,
  thumbnail: "https://example.com/t.jpg",
  thumbnailLocal: null,
  filePath: "C:/Videos/a.mp4",
  fileSize: 1000,
  mode: "video",
  quality: "1080p",
  finishedAt: 1_700_000_000,
  fileIdentity: { kind: "fileId", volume: 7, id: "42" },
  keptFiles: null,
  publishedSidecars: [],
  ...over,
});

function library() {
  const document = installFakeDom();
  let builds = 0;
  const cache = new TileCache();
  const build = () => {
    builds += 1;
    return document.createElement("li");
  };
  return {
    document,
    cache,
    tile: (it) => cache.get(it, build),
    get builds() {
      return builds;
    },
  };
}

test("an unchanged item keeps its tile and the keyboard focus on it", () => {
  const lib = library();
  const first = lib.tile(item());
  first.focus();
  const again = lib.tile(item());
  assert.equal(again, first);
  assert.equal(lib.builds, 1);
  assert.equal(lib.document.activeElement, first);
});

for (const [field, value] of [
  ["title", "Another title"],
  ["thumbnailLocal", "C:/thumbs/a.jpg"],
  ["thumbnail", "https://example.com/other.jpg"],
  ["fileSize", 2000],
  ["filePath", "D:/Moved/a.mp4"],
  ["quality", "720p"],
  ["finishedAt", 1_700_000_999],
  ["uploader", "Someone else"],
  ["duration", 62],
  ["mode", "audio"],
]) {
  test(`a changed ${field} rebuilds the tile once, then it is stable again`, () => {
    const lib = library();
    const first = lib.tile(item());
    const changed = lib.tile(item({ [field]: value }));
    assert.notEqual(changed, first);
    assert.equal(lib.builds, 2);
    assert.equal(lib.tile(item({ [field]: value })), changed);
    assert.equal(lib.builds, 2);
  });
}

test("a missing uploader or duration that appears later rebuilds (they were null before)", () => {
  const lib = library();
  lib.tile(item({ uploader: null, duration: null }));
  lib.tile(item());
  assert.equal(lib.builds, 2);
});

test("Delete File Too depends on proof of the file, so gaining or losing it rebuilds", () => {
  const lib = library();
  lib.tile(item({ fileIdentity: null }));
  lib.tile(item({ fileIdentity: { kind: "fileId", volume: 7, id: "42" } }));
  lib.tile(item({ fileIdentity: undefined }));
  assert.equal(lib.builds, 3);
});

test("which identity it is does not matter to the tile, so it does not rebuild", () => {
  const lib = library();
  const first = lib.tile(item());
  assert.equal(lib.tile(item({ fileIdentity: { kind: "content", size: 1000, sha256: "ab" } })), first);
  assert.equal(lib.builds, 1);
});

test("kept subtitle files appearing, or their number or folder changing, rebuild the tile", () => {
  const kept = (n, path = "C:/work/a") => ({ workspace: { path }, sources: Array.from({ length: n }, (_, i) => `s${i}.srt`) });
  const lib = library();
  const plain = lib.tile(item());
  const one = lib.tile(item({ keptFiles: kept(1) }));
  const two = lib.tile(item({ keptFiles: kept(2) }));
  const moved = lib.tile(item({ keptFiles: kept(2, "D:/work/a") }));
  assert.equal(new Set([plain, one, two, moved]).size, 4);
  assert.equal(lib.builds, 4);
  assert.equal(lib.tile(item({ keptFiles: kept(2, "D:/work/a") })), moved, "same record: same tile");
});

test("values that merely contain a separator cannot collide", () => {
  const lib = library();
  const a = lib.tile(item({ title: "A|B", thumbnailLocal: "C", thumbnail: null }));
  const b = lib.tile(item({ title: "A", thumbnailLocal: "B|C", thumbnail: null }));
  assert.notEqual(a, b);
  const c = lib.tile(item({ thumbnailLocal: null, thumbnail: "null" }));
  const d = lib.tile(item({ thumbnailLocal: null, thumbnail: null }));
  assert.notEqual(c, d);
  assert.notEqual(tileSignature(item({ title: "A|B", thumbnailLocal: "C" })), tileSignature(item({ title: "A", thumbnailLocal: "B|C" })));
});

test("a local thumbnail and a remote one are different tiles even with the same text", () => {
  const lib = library();
  const local = lib.tile(item({ thumbnailLocal: "x", thumbnail: null }));
  const remote = lib.tile(item({ thumbnailLocal: null, thumbnail: "x" }));
  assert.notEqual(local, remote);
});

test("what the tile does not show or use never rebuilds it: the link, and anything live", () => {
  const lib = library();
  const first = lib.tile(item());
  assert.equal(lib.tile({ ...item({ url: "https://example.com/changed" }), progress: 0.5, speed: "3 MB/s" }), first);
  assert.equal(lib.builds, 1);
});

test("items that left the library leave no cache entry", () => {
  const lib = library();
  lib.tile(item({ id: "a" }));
  lib.tile(item({ id: "b" }));
  lib.tile(item({ id: "c" }));
  assert.equal(lib.cache.size, 3);
  lib.cache.prune(new Set(["b"]));
  assert.equal(lib.cache.size, 1);
  lib.tile(item({ id: "a" }));
  assert.equal(lib.builds, 4, "a came back as a new tile");
});

test("tiles are kept by id: two items never share one", () => {
  const lib = library();
  assert.notEqual(lib.tile(item({ id: "a" })), lib.tile(item({ id: "b" })));
});

// The Library's grid after a refresh: tiles that stay must not be taken out and put back, which
// would drop the keyboard focus from the one a user is on.
function grid(count) {
  const document = installFakeDom();
  const ul = document.createElement("ul");
  const tiles = Array.from({ length: count }, (_, i) => {
    const li = document.createElement("li");
    li.id = `t${i}`;
    ul.appendChild(li);
    return li;
  });
  return { document, ul, tiles };
}

test("reconciling the grid keeps focus on a tile that stays, while others come, go and move", () => {
  const { document, ul, tiles } = grid(4);
  const fresh = document.createElement("li");
  tiles[1].focus();
  reconcileChildren(ul, [tiles[0], fresh, tiles[1], tiles[3]]); // tiles[2] left, fresh arrived
  assert.deepEqual(ul.children, [tiles[0], fresh, tiles[1], tiles[3]]);
  assert.equal(document.activeElement, tiles[1]);
  assert.equal(tiles[2].parentNode, null, "the departed tile is detached");
});

test("reconciling an unchanged grid touches nothing", () => {
  const { document, ul, tiles } = grid(3);
  tiles[2].focus();
  reconcileChildren(ul, [...tiles]);
  assert.deepEqual(ul.children, tiles);
  assert.equal(document.activeElement, tiles[2]);
});

test("reconciling to an empty grid removes every tile", () => {
  const { ul, tiles } = grid(2);
  reconcileChildren(ul, []);
  assert.deepEqual(ul.children, []);
  assert.ok(tiles.every((t) => t.parentNode === null));
});

test("a reorder puts tiles in the new order", () => {
  const { ul, tiles } = grid(3);
  reconcileChildren(ul, [tiles[2], tiles[0], tiles[1]]);
  assert.deepEqual(ul.children, [tiles[2], tiles[0], tiles[1]]);
});
