// Tests for StatusTransitions in src/queue-events.ts (compiled by "test:queue" in package.json).
import assert from "node:assert/strict";
import { test } from "node:test";

import { JobTable, StatusTransitions } from "../../.test-build/queue-events.js";

const job = (id, status, createdAt = 1) => ({ id, createdAt, status, progress: 0 });
const event = (j, revision) => ({ schemaVersion: 1, jobId: j.id, revision, job: j });

test("a job already failed at bootstrap does not notify again on a newer same-status event", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "failed"), job("b", "completed")]);
  assert.equal(transitions.observe(job("a", "failed")), null);
  assert.equal(transitions.observe(job("b", "completed")), null);
});

test("a repeated Failed or Completed event notifies once", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "downloading"), job("b", "processing")]);
  assert.equal(transitions.observe(job("a", "failed")), "failed");
  assert.equal(transitions.observe(job("a", "failed")), null);
  assert.equal(transitions.observe(job("b", "completed")), "completed");
  assert.equal(transitions.observe(job("b", "completed")), null);
});

test("newer events the table reports as same-status never notify", () => {
  const table = new JobTable();
  const transitions = new StatusTransitions();
  table.load([job("a", "downloading")], () => 1);
  transitions.seed(table.list());
  const seen = [];
  for (const [status, revision] of [["failed", 2], ["failed", 3], ["failed", 4]]) {
    const change = table.apply(event(job("a", status), revision));
    assert.equal(change.kind, "updated");
    seen.push(transitions.observe(table.get("a")));
  }
  assert.deepEqual(seen, ["failed", null, null]);
});

test("a retry that fails again is a new Failed", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "failed")]);
  assert.equal(transitions.observe(job("a", "failed")), null);
  assert.equal(transitions.observe(job("a", "queued")), null);
  assert.equal(transitions.observe(job("a", "downloading")), null);
  assert.equal(transitions.observe(job("a", "failed")), "failed");
  assert.equal(transitions.observe(job("a", "failed")), null);
});

test("a retry that completes notifies Completed", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "failed")]);
  transitions.observe(job("a", "queued"));
  assert.equal(transitions.observe(job("a", "completed")), "completed");
});

test("a job first seen already failed notifies once", () => {
  const transitions = new StatusTransitions();
  transitions.seed([]);
  assert.equal(transitions.observe(job("new", "failed")), "failed");
  assert.equal(transitions.observe(job("new", "failed")), null);
});

test("pruned ids are forgotten and others keep their state", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "failed"), job("b", "failed")]);
  transitions.prune(["a"]);
  assert.equal(transitions.size, 1);
  assert.equal(transitions.observe(job("b", "failed")), "failed", "b was removed, so it is unknown again");
  assert.equal(transitions.observe(job("a", "failed")), null, "a is untouched");
});

test("seeding replaces whatever was tracked before", () => {
  const transitions = new StatusTransitions();
  transitions.seed([job("a", "failed")]);
  transitions.seed([job("c", "queued")]);
  assert.equal(transitions.size, 1);
  assert.equal(transitions.observe(job("a", "failed")), "failed");
});
