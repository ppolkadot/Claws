import assert from "node:assert/strict";
import { test } from "node:test";
import { SpotifyMatchController, initialSpotifyQuery, isSpotifyInput } from "../../.test-build/spotify-match.js";

const id = "4cOdK2wGLETKBW3PvgPWqT";
const url = `https://open.spotify.com/track/${id}`;
const videoId = "abcdefghijk";
const otherId = "lmnopqrstuv";
const youtube = (id = videoId) => `https://www.youtube.com/watch?v=${id}`;
const track = (over = {}) => ({ url, id, title: "Song", artists: ["Artist"], duration: 200, artwork: null, ...over });
const candidate = (id = videoId) => ({ url: youtube(id), videoId: id, title: "Song", uploader: "Artist", duration: 200, thumbnail: null, score: 1, warnings: [] });
const probe = (over = {}) => ({ kind: "video", url: youtube(), title: "Song", uploader: "Artist", thumbnail: null, duration: 200, site: "YouTube", heights: [], subtitles: [], entries: [], audioAvailable: true, ...over });
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const setup = (over = {}, onChange) => {
  const calls = { resolve: [], search: [], probe: [] };
  const deps = {
    resolve: async (input) => { calls.resolve.push(input); return { track: track(), warnings: [] }; },
    search: async (song, query) => { calls.search.push([song, query]); return [candidate(), candidate(otherId)]; },
    probe: async (input) => { calls.probe.push(input); return probe({ url: input }); },
    ...over,
  };
  return { controller: new SpotifyMatchController(deps, onChange), calls };
};

test("Spotify routing uses exact hosts and catches unsupported Spotify forms", () => {
  for (const input of [url, "https://open.spotify.com/album/x", "https://spotify.link/abc", "spotify:track:abc", "http://open.spotify.com/track/x"]) assert.equal(isSpotifyInput(input), true, input);
  for (const input of ["https://open.spotify.com.evil.test/track/x", "https://evil.test/?next=https://open.spotify.com/track/x", "https://notspotify.link/x", youtube()]) assert.equal(isSpotifyInput(input), false, input);
});

test("initial queries use real metadata and do not invent an artist", () => {
  assert.equal(initialSpotifyQuery(track()), "Artist - Song audio");
  assert.equal(initialSpotifyQuery(track({ artists: [] })), "Song");
  assert.equal(initialSpotifyQuery(track({ title: null })), "");
});

test("resolution searches once with no default selection or downloadable preview", async () => {
  const { controller: c, calls } = setup();
  await c.open(url);
  assert.equal(c.state.stage, "choosing");
  assert.equal(c.state.query, "Artist - Song audio");
  assert.equal(calls.search.length, 1);
  assert.equal(c.state.selectedId, null);
  await c.confirm();
  assert.equal(calls.probe.length, 0);
  assert.equal(c.state.confirmed, null);
});

test("missing metadata permits manual query search without an initial request", async () => {
  const { controller: c, calls } = setup({ resolve: async () => ({ track: track({ title: null, artists: [] }), warnings: ["Metadata unavailable"] }) });
  await c.open(url);
  assert.equal(c.state.stage, "editable");
  assert.equal(calls.search.length, 0);
  c.setQuery("artist song");
  await c.search();
  assert.equal(calls.search.length, 1);
  assert.deepEqual(c.state.warnings, ["Metadata unavailable"]);
});

test("resolver failure for a full track still permits manual search; short-link failure is retryable", async () => {
  const { controller: c, calls } = setup({ resolve: async () => { throw new Error("offline"); } });
  await c.open(url);
  assert.equal(c.state.stage, "editable");
  assert.equal(c.state.track.id, id);
  assert.match(c.state.error, /Enter the artist and song title/);
  c.setQuery("manual song");
  await c.search();
  assert.equal(calls.search.length, 1);
  await c.open("https://spotify.link/short");
  assert.equal(c.state.stage, "failed");
  assert.equal(c.state.track, null);
  assert.match(c.state.error, /full Spotify track link/i);
});

test("blank and oversized literal queries never invoke search", async () => {
  const { controller: c, calls } = setup();
  await c.open(url);
  for (const value of [" ", "x".repeat(301)]) {
    c.setQuery(value);
    await c.search();
    assert.equal(calls.search.length, 1);
    assert.equal(c.state.stage, "editable");
    assert.ok(c.state.error);
  }
  c.setQuery("--literal https://example.com");
  await c.search();
  assert.equal(calls.search[1][1], "--literal https://example.com");
});

test("only an explicitly selected fresh same-video audio probe confirms", async () => {
  const { controller: c, calls } = setup();
  await c.open(url);
  c.select(otherId);
  await c.confirm();
  assert.deepEqual(calls.probe, [youtube(otherId)]);
  assert.equal(c.state.stage, "confirmed");
  assert.equal(c.state.confirmed.url, youtube(otherId));
  c.select(videoId);
  assert.equal(c.state.confirmed, null);
  assert.equal(c.state.stage, "choosing");
  await c.confirm();
  c.setQuery("new query");
  assert.equal(c.state.confirmed, null);
  assert.equal(c.state.selectedId, null);
  assert.deepEqual(c.state.candidates, []);
});

for (const result of [probe({ audioAvailable: false }), probe({ audioAvailable: null }), probe({ audioAvailable: undefined }), probe({ kind: "playlist" }), probe({ url: youtube(otherId) }), probe({ url: "https://evil.test/watch?v=abcdefghijk" })]) {
  test(`probe cannot confirm ${result.kind}/${result.url}/${result.audioAvailable}`, async () => {
    const { controller: c } = setup({ probe: async () => result });
    await c.open(url);
    c.select(videoId);
    await c.confirm();
    assert.equal(c.state.confirmed, null);
    assert.equal(c.state.stage, "choosing");
    assert.ok(c.state.error);
  });
}

test("rerunning search invalidates confirmation and selection immediately", async () => {
  const next = deferred();
  let searches = 0;
  const { controller: c } = setup({ search: async () => ++searches === 1 ? [candidate()] : next.promise });
  await c.open(url); c.select(videoId); await c.confirm();
  const pending = c.search();
  assert.equal(c.state.confirmed, null);
  assert.equal(c.state.selectedId, null);
  assert.equal(c.state.stage, "searching");
  next.resolve([]); await pending;
  assert.match(c.state.error, /No YouTube/i);
});

test("query edits invalidate a late resolve without overwriting the query", async () => {
  const pending = deferred();
  const { controller: c, calls } = setup({ resolve: () => pending.promise });
  const work = c.open(url);
  c.setQuery("my manual query");
  pending.resolve({ track: track(), warnings: [] }); await work;
  assert.equal(c.state.query, "my manual query");
  assert.equal(calls.search.length, 0);
  assert.equal(c.state.stage, "editable");
});

test("replacement input ignores late resolve", async () => {
  const pending = deferred(); let resolutions = 0;
  const { controller: c } = setup({ resolve: async () => ++resolutions === 1 ? pending.promise : { track: track({ title: "New" }), warnings: [] } });
  const old = c.open(url);
  await c.open(`${url}?new`);
  pending.resolve({ track: track({ title: "Old" }), warnings: [] }); await old;
  assert.equal(c.state.track.title, "New");
});

test("query changes and a newer search suppress older search replies", async () => {
  const older = deferred(), newer = deferred(); let searches = 0;
  const { controller: c } = setup({ search: async () => ++searches === 1 ? older.promise : newer.promise });
  const opening = c.open(url);
  await Promise.resolve(); await Promise.resolve();
  c.setQuery("new"); const searching = c.search();
  newer.resolve([candidate(otherId)]); await searching;
  older.resolve([candidate()]); await opening;
  assert.equal(c.state.candidates[0].videoId, otherId);
});

test("candidate selection suppresses a late probe and duplicate confirmations", async () => {
  const pending = deferred(); let probes = 0;
  const { controller: c } = setup({ probe: async () => { probes++; return pending.promise; } });
  await c.open(url); c.select(videoId);
  const first = c.confirm(); await c.confirm();
  assert.equal(probes, 1);
  c.select(otherId);
  pending.resolve(probe()); await first;
  assert.equal(c.state.confirmed, null);
  assert.equal(c.state.selectedId, otherId);
});

test("snapshot is detached immutable plain data and restore makes no requests", async () => {
  const { controller: c } = setup();
  await c.open(url); c.select(videoId); await c.confirm();
  const saved = c.snapshot();
  assert.deepEqual(JSON.parse(JSON.stringify(saved)), saved);
  assert.throws(() => { c.state.track.artists.push("changed"); }, TypeError);
  const { controller: restored, calls } = setup();
  restored.restore(saved);
  assert.equal(restored.state.stage, "confirmed");
  assert.equal(restored.state.confirmed.url, youtube());
  assert.deepEqual(calls, { resolve: [], search: [], probe: [] });
});

test("navigation saves pending work as settled retryable data", async () => {
  for (const operation of ["resolve", "search", "probe"]) {
    const pending = deferred(); const { controller: c } = setup({ [operation]: () => pending.promise });
    let work;
    if (operation === "resolve" || operation === "search") {
      work = c.open(url); await Promise.resolve(); await Promise.resolve();
    } else { await c.open(url); c.select(videoId); work = c.confirm(); }
    const saved = c.snapshot();
    assert.ok(!["resolving", "searching", "probing"].includes(saved.stage));
    assert.equal(saved.confirmed, null);
    assert.ok(saved.error);
    const { controller: back, calls } = setup(); back.restore(saved);
    assert.deepEqual(calls, { resolve: [], search: [], probe: [] });
    c.dispose(); pending.resolve(operation === "resolve" ? { track: track(), warnings: [] } : operation === "search" ? [] : probe()); await work;
  }
});

test("restore and dispose invalidate pending work and dispose suppresses callbacks", async () => {
  const pending = deferred(); const changes = [];
  const { controller: settled } = setup(); await settled.open(url);
  const { controller: c } = setup({ resolve: () => pending.promise }, (state) => changes.push(state));
  const work = c.open(url); c.restore(settled.snapshot());
  const count = changes.length; c.dispose(); c.setQuery("ignored");
  pending.resolve({ track: track({ title: "late" }), warnings: [] }); await work;
  assert.equal(changes.length, count);
  assert.equal(c.state.track.title, "Song");
});

test("restore rejects inconsistent confirmation data and leaves a usable review", async () => {
  const { controller: c } = setup(); await c.open(url); c.select(videoId); await c.confirm();
  const { controller: back } = setup();
  back.restore({ ...c.snapshot(), confirmed: null });
  assert.equal(back.state.stage, "choosing");
  await back.confirm();
  assert.equal(back.state.stage, "confirmed");
  back.restore({ ...c.snapshot(), selectedId: otherId });
  assert.equal(back.state.confirmed, null);
  assert.equal(back.state.stage, "choosing");
});

test("a replacement input suppresses a late selected-video probe", async () => {
  const pending = deferred();
  const { controller: c } = setup({ probe: () => pending.promise });
  await c.open(url); c.select(videoId); const work = c.confirm();
  await c.open(`${url}?replacement`);
  pending.resolve(probe()); await work;
  assert.equal(c.state.url, `${url}?replacement`);
  assert.equal(c.state.stage, "choosing");
  assert.equal(c.state.confirmed, null);
});

test("search results remain bounded distinct YouTube identities", async () => {
  const { controller: c } = setup({ search: async () => [
    { ...candidate(), url: "https://evil.test/watch?v=abcdefghijk" },
    ...[videoId, videoId, otherId, "12345678901", "12345678902", "12345678903", "12345678904"].map(candidate),
  ] });
  await c.open(url);
  assert.equal(c.state.candidates.length, 5);
  assert.equal(new Set(c.state.candidates.map((item) => item.videoId)).size, 5);
  assert.equal(c.state.selectedId, null);
});
