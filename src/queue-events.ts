// The download list as the backend describes it, built from revisioned events.
//
// Every observable change to a job carries a revision taken under the backend's jobs lock.
// Events can arrive late or twice (the enqueue reply and the jobs-added event describe the
// same jobs), so an event older than, or equal to, what is already known is ignored, and a
// removed job leaves a tombstone so a delayed update can never bring it back.
//
// Pure and DOM-free so it can be tested on its own (scripts/tests/queue-events.test.mjs).

export const EVENT_SCHEMA = 1;

/** Subscribe before bootstrap, then replay changes after its snapshot is installed. */
export class StartupEvents {
  private pending: Array<() => void> | null = [];

  accept(change: () => void) {
    if (this.pending) this.pending.push(change);
    else change();
  }

  flush() {
    const pending = this.pending ?? [];
    this.pending = null;
    for (const change of pending) change();
  }
}

export interface JobEvent<T> {
  schemaVersion: number;
  jobId: string;
  revision: number;
  job: T;
}

export interface JobsAdded<T> {
  schemaVersion: number;
  jobs: JobEvent<T>[];
}

export interface JobRemoved {
  schemaVersion: number;
  jobId: string;
  revision: number;
}

export interface Ordered {
  id: string;
  createdAt: number;
  status: string;
  /** Raised by "Move to Top"; a higher value is listed (and started) first. */
  priority?: number;
}

export type Change =
  /** A job not seen before: the list's structure changed. */
  | { kind: "added" }
  /** A newer state of a known job. `moved`: its place in the list changed (Move to Top). */
  | { kind: "updated"; statusChanged: boolean; moved: boolean }
  /** Stale, duplicate, removed or of an unknown schema. */
  | { kind: "ignored" };

const TOMBSTONE_LIMIT = 5000;

function before(a: Ordered, b: Ordered): boolean {
  const [pa, pb] = [a.priority ?? 0, b.priority ?? 0];
  if (pa !== pb) return pa > pb;
  return a.createdAt < b.createdAt || (a.createdAt === b.createdAt && a.id < b.id);
}

export class JobTable<T extends Ordered> {
  private jobs = new Map<string, T>();
  private revisions = new Map<string, number>();
  private tombstones = new Map<string, number>();
  /** Ids in display order (jobs moved to the top first, then creation time, then id). */
  private order: string[] = [];
  private listCache: T[] | null = null;
  /** Events refused because of their schema; the caller may report them. */
  rejected = 0;

  /** Replaces everything with a snapshot (the bootstrap). */
  load(jobs: T[], revisionOf: (job: T) => number) {
    this.jobs = new Map(jobs.map((job) => [job.id, job]));
    this.revisions = new Map(jobs.map((job) => [job.id, revisionOf(job)]));
    this.tombstones.clear();
    this.order = [...jobs].sort((a, b) => (before(a, b) ? -1 : before(b, a) ? 1 : 0)).map((job) => job.id);
    this.listCache = null;
  }

  get size(): number {
    return this.jobs.size;
  }

  get(id: string): T | undefined {
    return this.jobs.get(id);
  }

  values(): IterableIterator<T> {
    return this.jobs.values();
  }

  /** Jobs in display order. The same array is returned until the structure changes. */
  list(): T[] {
    if (!this.listCache) this.listCache = this.order.map((id) => this.jobs.get(id)!);
    return this.listCache;
  }

  apply(event: JobEvent<T>): Change {
    if (event.schemaVersion !== EVENT_SCHEMA || event.jobId !== event.job?.id) {
      this.rejected += 1;
      return { kind: "ignored" };
    }
    if (this.tombstones.has(event.jobId)) return { kind: "ignored" };
    const known = this.revisions.get(event.jobId);
    if (known !== undefined && event.revision <= known) return { kind: "ignored" };
    const previous = this.jobs.get(event.jobId);
    this.jobs.set(event.jobId, event.job);
    this.revisions.set(event.jobId, event.revision);
    if (!previous) {
      this.insert(event.job);
      return { kind: "added" };
    }
    const statusChanged = previous.status !== event.job.status;
    // The position follows creation time, which never changes, and the priority, which
    // only Move to Top changes: then the job is taken out and put back in its new place.
    if ((previous.priority ?? 0) !== (event.job.priority ?? 0)) {
      const index = this.order.indexOf(event.jobId);
      if (index >= 0) this.order.splice(index, 1);
      this.insert(event.job);
      return { kind: "updated", statusChanged, moved: true };
    }
    if (this.listCache) {
      const index = this.order.indexOf(event.jobId);
      if (index >= 0) this.listCache[index] = event.job;
    }
    return { kind: "updated", statusChanged, moved: false };
  }

  applyBatch(batch: JobsAdded<T>): { added: string[]; updated: string[]; statusChanged: boolean; moved: boolean } {
    const result = { added: [] as string[], updated: [] as string[], statusChanged: false, moved: false };
    if (batch.schemaVersion !== EVENT_SCHEMA) {
      this.rejected += 1;
      return result;
    }
    for (const event of batch.jobs) {
      const change = this.apply(event);
      if (change.kind === "added") result.added.push(event.jobId);
      else if (change.kind === "updated") {
        result.updated.push(event.jobId);
        result.statusChanged ||= change.statusChanged;
        result.moved ||= change.moved;
      }
    }
    return result;
  }

  remove(event: JobRemoved): boolean {
    if (event.schemaVersion !== EVENT_SCHEMA) {
      this.rejected += 1;
      return false;
    }
    const known = this.revisions.get(event.jobId);
    // Ids are never reused, but a stale removal is still ignored, symmetric with apply().
    if (known !== undefined && event.revision < known) return false;
    this.tombstones.set(event.jobId, event.revision);
    if (this.tombstones.size > TOMBSTONE_LIMIT) {
      const oldest = this.tombstones.keys().next().value;
      if (oldest !== undefined) this.tombstones.delete(oldest);
    }
    this.revisions.delete(event.jobId);
    if (!this.jobs.delete(event.jobId)) return false;
    this.order.splice(this.order.indexOf(event.jobId), 1);
    this.listCache = null;
    return true;
  }

  private insert(job: T) {
    let low = 0;
    let high = this.order.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (before(this.jobs.get(this.order[mid])!, job)) low = mid + 1;
      else high = mid;
    }
    this.order.splice(low, 0, job.id);
    this.listCache = null;
  }
}

/** The job statuses that earn a notice. */
export type NoticeStatus = "completed" | "failed";

/**
 * Turns status changes into notices. The backend can send a newer event for a job whose
 * status has not changed (a failed job that is updated again), and the store passes it on,
 * so a notice follows a transition into Completed or Failed, never the status itself.
 * A job that is retried leaves Failed first, so failing again is a new transition.
 */
export class StatusTransitions {
  private statuses = new Map<string, string>();

  /** Starts from the bootstrap snapshot: what is already finished or failed is not news. */
  seed(jobs: Iterable<{ id: string; status: string }>) {
    this.statuses = new Map();
    for (const job of jobs) this.statuses.set(job.id, job.status);
  }

  get size(): number {
    return this.statuses.size;
  }

  /** Records a job's latest state; returns the status to announce, if it newly arrived. */
  observe(job: { id: string; status: string }): NoticeStatus | null {
    const previous = this.statuses.get(job.id);
    this.statuses.set(job.id, job.status);
    if (previous === job.status) return null;
    return job.status === "completed" || job.status === "failed" ? job.status : null;
  }

  /** Forgets jobs that are no longer in the list. */
  prune(ids: Iterable<string>) {
    const keep = new Set(ids);
    for (const id of this.statuses.keys()) {
      if (!keep.has(id)) this.statuses.delete(id);
    }
  }
}
