// Tests for src/download-input.ts, compiled first into .test-build/ (see package.json
// "test:logic"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/download-input.ts
//   node --test scripts/tests/download-input.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  ALREADY_QUEUED,
  LinkBatch,
  MAX_ENQUEUE,
  MAX_LINKS,
  describeEnqueue,
  isBatchInput,
  parseLinks,
  planChunks,
} from "../../.test-build/download-input.js";

// ---- Parser ------------------------------------------------------------------------

test("splits LF, CRLF and lone CR lines and keeps physical line numbers", () => {
  const parsed = parseLinks("https://a.example/1\r\nhttps://b.example/2\nhttps://c.example/3\rhttps://d.example/4");
  assert.deepEqual(
    parsed.links.map((l) => [l.line, l.url]),
    [
      [1, "https://a.example/1"],
      [2, "https://b.example/2"],
      [3, "https://c.example/3"],
      [4, "https://d.example/4"],
    ],
  );
  assert.equal(parsed.invalid.length, 0);
  assert.equal(parsed.overLimit, false);
});

test("skips blank and whitespace-only lines and trims each line", () => {
  const parsed = parseLinks("\n\n   https://a.example/x  \r\n\t\r\n \nhttps://b.example/y\n\n");
  assert.deepEqual(
    parsed.links.map((l) => [l.line, l.url]),
    [
      [3, "https://a.example/x"],
      [6, "https://b.example/y"],
    ],
  );
  assert.equal(parsed.lineCount, 2);
  assert.equal(parsed.invalid.length, 0);
});

test("reports exact duplicates with both line numbers instead of dropping them silently", () => {
  const parsed = parseLinks("https://a.example/v\nhttps://b.example/v\nhttps://a.example/v\nHTTPS://A.EXAMPLE/v");
  assert.deepEqual(parsed.links.map((l) => l.url), ["https://a.example/v", "https://b.example/v"]);
  assert.deepEqual(parsed.duplicates, [
    { line: 3, firstLine: 1, url: "https://a.example/v" },
    // The scheme and host are case-insensitive, so URL.href normalises them.
    { line: 4, firstLine: 1, url: "https://a.example/v" },
  ]);
});

test("distinct paths and signed queries are never merged", () => {
  const parsed = parseLinks(
    [
      "https://cdn.example/file.mp4?sig=AAA&exp=1",
      "https://cdn.example/file.mp4?sig=BBB&exp=1",
      "https://cdn.example/File.mp4?sig=AAA&exp=1",
      "https://cdn.example/file.mp4?exp=1&sig=AAA",
    ].join("\n"),
  );
  assert.equal(parsed.links.length, 4);
  assert.equal(parsed.duplicates.length, 0);
});

test("preserves query parameters and fragments exactly", () => {
  const raw = "https://www.example.com/watch?v=abc_DEF-1&list=PL1x&t=30s&sig=AbC%2Fd%3D#chapter-2";
  const parsed = parseLinks(raw);
  assert.equal(parsed.links[0].url, raw);
});

test("rejects malformed schemes and lines with more than one link, with line and reason", () => {
  const parsed = parseLinks(
    [
      "ftp://files.example/a.mp4",
      "javascript:alert(1)",
      "example.com/video",
      "https://a.example/1 https://b.example/2",
      "https://",
      "mailto:someone@example.com",
      "file:///C:/video.mp4",
    ].join("\n"),
  );
  assert.equal(parsed.links.length, 0);
  assert.deepEqual(
    parsed.invalid.map((i) => i.line),
    [1, 2, 3, 4, 5, 6, 7],
  );
  const reason = (line) => parsed.invalid.find((i) => i.line === line).reason;
  assert.match(reason(1), /http and https/);
  assert.match(reason(2), /http and https/);
  assert.match(reason(3), /does not look like a link/);
  assert.match(reason(4), /one link on each line/);
  assert.match(reason(5), /does not look like a link/);
  assert.match(reason(7), /http and https/);
  assert.equal(parsed.invalid[0].text, "ftp://files.example/a.mp4");
});

test("one invalid line among valid links keeps every valid link", () => {
  const parsed = parseLinks("https://a.example/1\nnot a link\nhttps://b.example/2\nhttps://c.example/3");
  assert.deepEqual(parsed.links.map((l) => l.line), [1, 3, 4]);
  assert.deepEqual(parsed.invalid, [{ line: 2, text: "not a link", reason: parsed.invalid[0].reason }]);
  assert.equal(isBatchInput(parsed), true);
});

test("500 links are accepted; 501 are refused with the limit named, never truncated", () => {
  const lines = (n) => Array.from({ length: n }, (_, i) => `https://example.com/v/${i}`).join("\n");
  const ok = parseLinks(lines(500));
  assert.equal(MAX_LINKS, 500);
  assert.equal(ok.links.length, 500);
  assert.equal(ok.overLimit, false);
  const over = parseLinks(lines(501));
  assert.equal(over.overLimit, true);
  assert.equal(over.links.length, 501, "the parse reports every link; the caller refuses the batch");
  // Duplicates do not count towards the limit.
  const dupes = parseLinks(lines(500) + "\nhttps://example.com/v/0");
  assert.equal(dupes.overLimit, false);
  assert.equal(dupes.duplicates.length, 1);
});

test("a single link keeps the single-link flow; several lines make a batch", () => {
  assert.equal(isBatchInput(parseLinks("https://a.example/1")), false);
  assert.equal(isBatchInput(parseLinks("\n  https://a.example/1 \n\n")), false);
  // The same link twice is still one link.
  assert.equal(isBatchInput(parseLinks("https://a.example/1\nhttps://a.example/1")), false);
  assert.equal(isBatchInput(parseLinks("https://a.example/1\nhttps://a.example/2")), true);
  // One link plus a line that is not one is still one link: it gets the single-link
  // preview (with its real qualities and subtitles), and the skipped line is reported there.
  assert.equal(isBatchInput(parseLinks("https://a.example/1\nftp://x")), false);
  assert.equal(isBatchInput(parseLinks("https://a.example/1\nhttps//typo\nhttps://a.example/1")), false);
  assert.equal(isBatchInput(parseLinks("not a link")), false);
  // Several lines and no link at all: the batch path, which names each refused line.
  assert.equal(isBatchInput(parseLinks("ftp://x\nnot a link")), true);
});

// ---- Lookup scheduler --------------------------------------------------------------

/** A probe whose replies the test controls. */
function controlledProbe() {
  const calls = [];
  let active = 0;
  let maxActive = 0;
  const probe = (url) => {
    active += 1;
    maxActive = Math.max(maxActive, active);
    let resolve;
    let reject;
    const promise = new Promise((res, rej) => {
      resolve = res;
      reject = rej;
    }).finally(() => (active -= 1));
    const call = { url, resolve: (r) => resolve(r), reject: (e) => reject(e) };
    calls.push(call);
    return promise;
  };
  return {
    calls,
    probe,
    get active() {
      return active;
    },
    get maxActive() {
      return maxActive;
    },
  };
}

const video = (url, extra = {}) => ({ kind: "video", url, title: `Title of ${url}`, thumbnail: null, uploader: "U", duration: 60, entries: [], ...extra });
const playlist = (url, n) => ({
  kind: "playlist",
  url,
  title: `Playlist ${url}`,
  thumbnail: null,
  uploader: "U",
  duration: null,
  entries: Array.from({ length: n }, (_, i) => ({ url: `${url}/e${i}`, playlistIndex: null, title: `Entry ${i}`, thumbnail: null, uploader: null, duration: 10 })),
});
const links = (n, prefix = "https://example.com/v/") => Array.from({ length: n }, (_, i) => ({ url: `${prefix}${i}`, line: i + 1 }));
const flush = () => new Promise((resolve) => setImmediate(resolve));
const statuses = (batch) => batch.rows.map((r) => r.status);

test("500 links: never more than three lookups at once, and every row finishes", async () => {
  let active = 0;
  let maxActive = 0;
  let calls = 0;
  const probe = async (url) => {
    active += 1;
    calls += 1;
    maxActive = Math.max(maxActive, active);
    // Varying, out-of-order completion.
    await new Promise((resolve) => setTimeout(resolve, (url.length * 7 + calls * 13) % 5));
    active -= 1;
    if (url.endsWith("7")) throw "Unsupported URL";
    return video(url);
  };
  const changes = [];
  const batch = new LinkBatch(probe, { onRow: (row) => changes.push(row.id) });
  batch.start(links(500));
  assert.equal(batch.rows.length, 500);
  while (batch.rows.some((r) => r.status === "waiting" || r.status === "looking")) await new Promise((r) => setTimeout(r, 2));
  assert.ok(maxActive <= 3, `max concurrent lookups was ${maxActive}`);
  assert.equal(maxActive, 3);
  assert.equal(calls, 500, "each link is looked up exactly once");
  const failed = batch.rows.filter((r) => r.status === "failed");
  assert.equal(failed.length, 50);
  assert.ok(failed.every((r) => r.error === "Unsupported URL" && !r.selected));
  assert.ok(batch.rows.filter((r) => r.status === "ready").every((r) => r.selected));
  assert.equal(batch.inFlight, 0);
  assert.ok(changes.length >= 1000, "every row reports looking and its result");
});

test("replies arriving out of order land on their own rows", async () => {
  const fake = controlledProbe();
  const batch = new LinkBatch(fake.probe);
  batch.start(links(3));
  assert.deepEqual(statuses(batch), ["looking", "looking", "looking"]);
  fake.calls[2].resolve(video(fake.calls[2].url));
  fake.calls[0].resolve(video(fake.calls[0].url));
  await flush();
  assert.deepEqual(statuses(batch), ["ready", "looking", "ready"]);
  assert.equal(batch.rows[0].result.url, "https://example.com/v/0");
  assert.equal(batch.rows[2].result.url, "https://example.com/v/2");
});

test("a superseded submission's replies are ignored and still count towards the limit", async () => {
  const fake = controlledProbe();
  const seen = [];
  const batch = new LinkBatch(fake.probe, { onRow: (row) => seen.push(row) });
  batch.start(links(5, "https://old.example/"));
  assert.equal(fake.calls.length, 3);
  const oldRows = [...batch.rows];
  batch.start(links(2, "https://new.example/"));
  assert.deepEqual(statuses(batch), ["waiting", "waiting"]);
  assert.equal(fake.calls.length, 3, "the old lookups are still running, so nothing new starts yet");
  seen.length = 0;
  fake.calls[0].resolve(video("https://old.example/0"));
  await flush();
  // The old reply is dropped and frees a slot for the new submission.
  assert.ok(seen.every((row) => row.url.startsWith("https://new.example/")));
  assert.equal(oldRows[0].status, "cancelled");
  assert.equal(oldRows[0].result, null);
  assert.deepEqual(statuses(batch), ["looking", "waiting"]);
  fake.calls[1].reject("old failure");
  fake.calls[2].resolve(video("https://old.example/2"));
  await flush();
  assert.deepEqual(statuses(batch), ["looking", "looking"]);
  assert.ok(fake.maxActive <= 3);
  fake.calls[3].resolve(video("https://new.example/0"));
  fake.calls[4].resolve(video("https://new.example/1"));
  await flush();
  assert.deepEqual(statuses(batch), ["ready", "ready"]);
  assert.equal(batch.rows[0].result.url, "https://new.example/0");
});

test("cancel stops scheduling, ignores late replies, and a row can be looked up again", async () => {
  const fake = controlledProbe();
  const batch = new LinkBatch(fake.probe);
  batch.start(links(10));
  assert.equal(fake.calls.length, 3);
  batch.cancel();
  assert.ok(batch.rows.every((r) => r.status === "cancelled"));
  fake.calls[0].resolve(video(fake.calls[0].url));
  fake.calls[1].reject("late failure");
  await flush();
  assert.ok(batch.rows.every((r) => r.status === "cancelled" && r.result === null && r.error === null));
  assert.equal(fake.calls.length, 3, "no new lookups after cancel");
  assert.equal(batch.inFlight, 1, "the one backend lookup still running is counted");
  batch.retry(batch.rows[5].id);
  assert.equal(batch.rows[5].status, "looking");
  assert.equal(fake.calls.length, 4);
  fake.calls[3].resolve(video("https://example.com/v/5"));
  await flush();
  assert.equal(batch.rows[5].status, "ready");
  assert.equal(batch.rows[5].selected, true);
  assert.equal(batch.rows.filter((r) => r.status === "cancelled").length, 9);
});

test("duplicate replies for one row: only the latest lookup's reply is applied", async () => {
  const fake = controlledProbe();
  const batch = new LinkBatch(fake.probe);
  batch.start(links(1));
  const row = batch.rows[0];
  batch.cancelRow(row.id);
  batch.retry(row.id);
  assert.equal(fake.calls.length, 2);
  // The new lookup answers first, then the old one: the old one must not overwrite it.
  fake.calls[1].resolve(video(row.url, { title: "fresh" }));
  await flush();
  fake.calls[0].resolve(video(row.url, { title: "stale" }));
  await flush();
  assert.equal(row.status, "ready");
  assert.equal(row.result.title, "fresh");
  // And the reverse order: the old reply arrives while the new lookup is still running.
  batch.start(links(1));
  const again = batch.rows[0];
  assert.equal(fake.calls.length, 3);
  batch.cancelRow(again.id);
  batch.retry(again.id);
  assert.equal(fake.calls.length, 4);
  fake.calls[2].resolve(video(again.url, { title: "stale" }));
  await flush();
  assert.equal(again.status, "looking", "the old reply does not finish the new lookup");
  assert.equal(again.result, null);
  fake.calls[3].resolve(video(again.url, { title: "newest" }));
  await flush();
  assert.equal(again.status, "ready");
  assert.equal(again.result.title, "newest");
  // A ready row is neither cancelled nor looked up again.
  batch.cancelRow(again.id);
  batch.retry(again.id);
  assert.equal(again.status, "ready");
  assert.equal(fake.calls.length, 4);
});

test("partial failure keeps successful rows; a failed row is retried on its own", async () => {
  const fake = controlledProbe();
  const batch = new LinkBatch(fake.probe);
  batch.start(links(4));
  fake.calls[0].resolve(video("https://example.com/v/0"));
  fake.calls[1].reject("This video is private.");
  fake.calls[2].resolve(video("https://example.com/v/2"));
  await flush();
  fake.calls[3].resolve(video("https://example.com/v/3"));
  await flush();
  assert.deepEqual(statuses(batch), ["ready", "failed", "ready", "ready"]);
  const failed = batch.rows[1];
  assert.equal(failed.error, "This video is private.");
  batch.setSelected(failed.id, true);
  assert.equal(failed.selected, false, "a failed row cannot be selected");
  assert.equal(batch.counts().selectedItems, 3);
  batch.retry(failed.id);
  assert.equal(fake.calls.length, 5);
  assert.equal(fake.calls[4].url, "https://example.com/v/1");
  assert.deepEqual(statuses(batch), ["ready", "looking", "ready", "ready"]);
  fake.calls[4].resolve(video("https://example.com/v/1"));
  await flush();
  assert.deepEqual(statuses(batch), ["ready", "ready", "ready", "ready"]);
  assert.equal(failed.error, null);
  assert.equal(batch.counts().selectedItems, 4);
  // Retrying a ready row does nothing.
  batch.retry(batch.rows[0].id);
  assert.equal(fake.calls.length, 5);
});

test("an Error, a string and anything else become readable messages", async () => {
  const replies = [new Error("boom"), "plain", { odd: true }];
  const batch = new LinkBatch(async () => {
    throw replies.shift();
  });
  batch.start(links(3));
  await flush();
  await flush();
  assert.deepEqual(
    batch.rows.map((r) => r.error),
    ["boom", "plain", "Something went wrong."],
  );
});

test("counts summarise each status and the selected videos", async () => {
  const fake = controlledProbe();
  const batch = new LinkBatch(fake.probe);
  batch.start(links(6));
  fake.calls[0].resolve(playlist("https://example.com/v/0", 4));
  fake.calls[1].reject("nope");
  await flush();
  const counts = batch.counts();
  assert.deepEqual(
    { waiting: counts.waiting, looking: counts.looking, ready: counts.ready, failed: counts.failed, selectedRows: counts.selectedRows, selectedItems: counts.selectedItems },
    { waiting: 1, looking: 3, ready: 1, failed: 1, selectedRows: 1, selectedItems: 4 },
  );
});

// ---- Enqueue -----------------------------------------------------------------------

test("planChunks keeps rows together and splits only a row larger than the limit", () => {
  const rows = [
    { id: 1, items: Array.from({ length: 300 }, (_, i) => ({ url: `a${i}` })) },
    { id: 2, items: Array.from({ length: 300 }, (_, i) => ({ url: `b${i}` })) },
    { id: 3, items: Array.from({ length: 700 }, (_, i) => ({ url: `c${i}` })) },
    { id: 4, items: [{ url: "d" }] },
  ];
  const chunks = planChunks(rows, 500);
  assert.equal(MAX_ENQUEUE, 500);
  assert.ok(chunks.every((c) => c.items.length <= 500 && c.items.length > 0));
  assert.deepEqual(
    chunks.map((c) => [c.items.length, c.rowIds]),
    [
      [300, [1]],
      [300, [2]],
      [500, [3]],
      [201, [3, 4]],
    ],
  );
  assert.equal(chunks.flatMap((c) => c.items).length, 1301);
});

async function readyBatch(results) {
  let i = 0;
  const batch = new LinkBatch(async () => {
    const r = results[i++];
    if (typeof r === "string") throw r;
    return r;
  });
  batch.start(results.map((_, n) => ({ url: `https://example.com/in/${n}`, line: n + 1 })));
  for (let n = 0; n < 10; n++) await flush();
  return batch;
}

test("a mixed batch enqueues valid selected entries exactly once, even with repeated clicks", async () => {
  const batch = await readyBatch([video("https://v.example/1"), "Unsupported URL", playlist("https://p.example/list", 3), video("https://v.example/4")]);
  batch.setSelected(batch.rows[3].id, false);
  const sent = [];
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const send = async (items) => {
    sent.push(items);
    await gate;
    return { jobs: items.map((item) => ({ jobId: item.url })) };
  };
  const first = batch.enqueue(send);
  const second = batch.enqueue(send);
  assert.equal(batch.submitting, true);
  assert.equal(await second, null, "a second click while adding does nothing");
  release();
  const summary = await first;
  assert.equal(sent.length, 1);
  assert.deepEqual(
    sent[0].map((item) => [item.url, item.playlistIndex ?? null]),
    [
      ["https://v.example/1", null],
      ["https://p.example/list/e0", null],
      ["https://p.example/list/e1", null],
      ["https://p.example/list/e2", null],
    ],
  );
  assert.equal(sent[0][0].title, "Title of https://v.example/1");
  assert.deepEqual({ added: summary.added, skipped: summary.skipped, failed: summary.failed }, { added: 4, skipped: 0, failed: 0 });
  assert.equal(batch.rows[0].added, true);
  assert.equal(batch.rows[0].selected, false);
  assert.equal(batch.rows[2].added, true);
  assert.equal(batch.rows[1].added, false);
  assert.equal(batch.rows[3].added, false);
  // Added rows cannot be selected or sent again.
  batch.setSelected(batch.rows[0].id, true);
  assert.equal(batch.rows[0].selected, false);
  assert.equal(await batch.enqueue(send), null);
  assert.equal(sent.length, 1);
  assert.equal(batch.submitting, false);
});

test("backend skips are reported, and a refused chunk leaves its rows selected to retry", async () => {
  const results = [playlist("https://p.example/big", 450), video("https://v.example/a"), playlist("https://p.example/two", 100), video("https://v.example/b")];
  const batch = await readyBatch(results);
  const sent = [];
  let call = 0;
  const send = async (items) => {
    sent.push(items.map((i) => i.url));
    call += 1;
    if (call === 2) throw "Claws is closing.";
    // The backend skips links already queued with the same options.
    return { jobs: items.slice(0, items.length - 2) };
  };
  const summary = await batch.enqueue(send);
  assert.deepEqual(sent.map((s) => s.length), [451, 101]);
  assert.deepEqual(
    { added: summary.added, skipped: summary.skipped, failed: summary.failed, rowsAdded: summary.rowsAdded, rowsFailed: summary.rowsFailed },
    { added: 449, skipped: 2, failed: 101, rowsAdded: 2, rowsFailed: 2 },
  );
  assert.deepEqual(summary.errors, ["Claws is closing."]);
  assert.deepEqual(batch.rows.map((r) => r.added), [true, true, false, false]);
  assert.equal(batch.rows[2].addError, "Claws is closing.");
  assert.equal(batch.rows[2].selected, true);
  // Retrying adds only what is left.
  const retry = await batch.enqueue(async (items) => {
    sent.push(items.map((i) => i.url));
    return { jobs: items };
  });
  assert.equal(sent.length, 3);
  assert.equal(sent[2].length, 101);
  assert.equal(retry.added, 101);
  assert.ok(batch.rows.every((r) => r.added && r.addError === null));
});

test("a cleared batch ignores everything that arrives afterwards", async () => {
  const fake = controlledProbe();
  const changes = [];
  const batch = new LinkBatch(fake.probe, { onRow: (row) => changes.push(row) });
  batch.start(links(4));
  batch.clear();
  assert.equal(batch.rows.length, 0);
  changes.length = 0;
  fake.calls.forEach((c) => c.resolve(video(c.url)));
  await flush();
  assert.equal(changes.length, 0);
  assert.equal(fake.calls.length, 3);
  assert.equal(batch.inFlight, 0);
});

test("describeEnqueue says what happened in plain words", () => {
  const base = { added: 0, skipped: 0, failed: 0, rowsAdded: 0, rowsFailed: 0, errors: [] };
  assert.equal(describeEnqueue({ ...base, added: 1, rowsAdded: 1 }), "Added 1 video to the queue.");
  assert.equal(describeEnqueue({ ...base, added: 12, rowsAdded: 3 }), "Added 12 videos to the queue.");
  assert.equal(
    describeEnqueue({ ...base, added: 10, skipped: 2, rowsAdded: 3 }),
    "Added 10 videos to the queue. 2 were already queued or listed twice.",
  );
  assert.equal(
    describeEnqueue({ ...base, added: 3, failed: 4, rowsAdded: 1, rowsFailed: 2, errors: ["Claws is closing."] }),
    "Added 3 videos to the queue. 4 could not be added: Claws is closing.",
  );
  assert.equal(
    describeEnqueue({ ...base, failed: 1, rowsFailed: 1, errors: ["That is already in the queue with the same options."] }),
    "1 video could not be added: That is already in the queue with the same options.",
  );
});

test("a call refused because every item is already queued counts as skipped, not failed", async () => {
  const batch = await readyBatch([video("https://v.example/a"), video("https://v.example/b"), video("https://v.example/c")]);
  let calls = 0;
  const summary = await batch.enqueue(async () => {
    calls += 1;
    // What downloader.rs enqueue returns when no item in the call is new.
    throw ALREADY_QUEUED;
  });
  assert.equal(calls, 1);
  assert.deepEqual(
    { added: summary.added, skipped: summary.skipped, failed: summary.failed, rowsAdded: summary.rowsAdded, rowsFailed: summary.rowsFailed, errors: summary.errors },
    { added: 0, skipped: 3, failed: 0, rowsAdded: 3, rowsFailed: 0, errors: [] },
  );
  assert.ok(batch.rows.every((r) => r.added && !r.selected && r.addError === null));
  assert.equal(describeEnqueue(summary), "3 were already queued or listed twice.");
  // Nothing is left to send, so another click sends nothing.
  assert.equal(await batch.enqueue(async () => assert.fail("nothing should be sent")), null);
});

test("an already-queued chunk is skipped the same way however the rows are packed", async () => {
  // 300 already-queued entries get a call of their own; the new video gets another.
  const batch = await readyBatch([playlist("https://p.example/queued", 300), playlist("https://p.example/new", 250)]);
  const summary = await batch.enqueue(async (items) => {
    if (items[0].url.startsWith("https://p.example/queued")) throw ALREADY_QUEUED;
    return { jobs: items };
  });
  assert.deepEqual({ added: summary.added, skipped: summary.skipped, failed: summary.failed }, { added: 250, skipped: 300, failed: 0 });
  assert.ok(batch.rows.every((r) => r.added && r.addError === null));
});

test("a row split across calls resends only the part that was refused, so nothing is queued twice", async () => {
  const batch = await readyBatch([playlist("https://p.example/huge", 600)]);
  const sent = [];
  let call = 0;
  const first = await batch.enqueue(async (items) => {
    sent.push(items.map((i) => i.url));
    call += 1;
    if (call === 2) throw "Claws is closing.";
    return { jobs: items };
  });
  assert.deepEqual(sent.map((s) => s.length), [500, 100]);
  assert.deepEqual({ added: first.added, failed: first.failed, rowsFailed: first.rowsFailed }, { added: 500, failed: 100, rowsFailed: 1 });
  const row = batch.rows[0];
  assert.equal(row.added, false);
  assert.equal(row.addError, "Claws is closing.");
  assert.equal(row.selected, true);
  assert.equal(batch.counts().selectedItems, 100, "only the refused part is still to add");
  // The first 500 may have finished meanwhile; they must not be sent again.
  const retry = await batch.enqueue(async (items) => {
    sent.push(items.map((i) => i.url));
    return { jobs: items };
  });
  assert.equal(sent.length, 3);
  assert.deepEqual(sent[2], sent[1], "the retry sends exactly the refused entries");
  assert.equal(retry.added, 100);
  assert.equal(retry.failed, 0);
  assert.equal(row.added, true);
  assert.equal(row.addError, null);
  const everySent = sent[0].concat(sent[2]);
  assert.equal(new Set(everySent).size, 600);
});

test("ALREADY_QUEUED is the exact message the backend's enqueue returns", () => {
  const source = readFileSync(new URL("../../src-tauri/src/downloader.rs", import.meta.url), "utf8");
  assert.ok(source.includes(`"${ALREADY_QUEUED}"`), "downloader.rs enqueue must refuse with ALREADY_QUEUED, word for word");
});
