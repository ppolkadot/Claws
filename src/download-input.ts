/** Links looked up from one submission. A longer list is refused, never cut short. */
export const MAX_LINKS = 500;
/** Lookups running at once; each one is a yt-dlp process. */
export const LOOKUP_CONCURRENCY = 3;
/** Items the backend accepts in one enqueue call (downloader.rs MAX_BATCH). */
export const MAX_ENQUEUE = 500;
/**
 * downloader.rs enqueue refuses a call with these exact words when no item in it is new:
 * every one is already waiting or running with the same options. For a batch that means
 * those items are in the queue, not that adding them failed (a Node test checks the text).
 */
export const ALREADY_QUEUED = "That is already in the queue with the same options.";

export interface ParsedLink {
  /** Normalised by URL.href: scheme and host lower-cased; path and query kept as typed. */
  url: string;
  /** 1-based, counting blank lines, so it matches what the person sees. */
  line: number;
}

export interface DuplicateLink {
  url: string;
  line: number;
  firstLine: number;
}

export interface InvalidLine {
  line: number;
  text: string;
  reason: string;
}

export interface ParsedInput {
  links: ParsedLink[];
  duplicates: DuplicateLink[];
  invalid: InvalidLine[];
  /** More than the limit of distinct links: the caller must refuse the batch. */
  overLimit: boolean;
  lineCount: number;
}

// The same wording as the backend's validate_url (probe.rs).
const NOT_A_LINK = "That does not look like a link. Paste a full http(s) address.";
const WRONG_SCHEME = "Only http and https links are supported.";
const ONE_PER_LINE = "Put one link on each line.";

function check(text: string): { url: string } | { reason: string } {
  if (/\s/.test(text)) return { reason: ONE_PER_LINE };
  let parsed: URL;
  try {
    parsed = new URL(text);
  } catch {
    return { reason: NOT_A_LINK };
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return { reason: WRONG_SCHEME };
  if (!parsed.hostname) return { reason: NOT_A_LINK };
  return { url: parsed.href };
}

/**
 * One http(s) link per line. Blank lines are skipped; repeated links (identical after
 * URL.href normalisation) and lines that are not a link are reported with their line
 * numbers. Distinct queries, such as two signed links, are never treated as the same.
 */
export function parseLinks(text: string, limit = MAX_LINKS): ParsedInput {
  const links: ParsedLink[] = [];
  const duplicates: DuplicateLink[] = [];
  const invalid: InvalidLine[] = [];
  const first = new Map<string, number>();
  let lineCount = 0;
  text.split(/\r\n|\r|\n/).forEach((raw, index) => {
    const line = index + 1;
    const trimmed = raw.trim();
    if (!trimmed) return;
    lineCount += 1;
    const result = check(trimmed);
    if ("reason" in result) {
      invalid.push({ line, text: trimmed, reason: result.reason });
      return;
    }
    const seen = first.get(result.url);
    if (seen !== undefined) {
      duplicates.push({ url: result.url, line, firstLine: seen });
      return;
    }
    first.set(result.url, line);
    links.push({ url: result.url, line });
  });
  return { links, duplicates, invalid, overLimit: links.length > limit, lineCount };
}

/**
 * A single link (even repeated, or beside lines that are not links) keeps the single-link
 * preview, which reports any skipped lines; two or more links are a batch. Several lines
 * with no link at all also go the batch way, so each refused line is named.
 */
export function isBatchInput(parsed: ParsedInput): boolean {
  if (parsed.lineCount <= 1) return false;
  return parsed.links.length !== 1;
}


/** The parts of a lookup result a batch uses (a subset of api.ts ProbeResult). */
export interface LookupEntry {
  url: string;
  playlistIndex: number | null;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
}

export interface LookupResult {
  kind: "video" | "playlist";
  url: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
  entries: LookupEntry[];
}

/** What one queue item needs (the same shape as api.ts EnqueueItem). */
export interface BatchItem {
  url: string;
  playlistIndex?: number | null;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
}

export type LookupStatus = "waiting" | "looking" | "ready" | "failed" | "cancelled";

export interface BatchRow<R extends LookupResult = LookupResult> {
  readonly id: number;
  readonly url: string;
  readonly line: number;
  status: LookupStatus;
  result: R | null;
  error: string | null;
  selected: boolean;
  /** Already sent to the queue; it cannot be sent again. */
  added: boolean;
  /** Why the last attempt to add it was refused; it stays selected to try again. */
  addError: string | null;
  /**
   * Items the queue has accepted (created, or skipped as already queued), by itemKey. A
   * playlist larger than one enqueue call can be partly accepted; trying again sends only
   * the rest, so nothing accepted is queued a second time.
   */
  accepted: Set<string>;
  /** Changes whenever a lookup starts or is cancelled; a reply for an older token is ignored. */
  token: number;
}

/** The queue items a looked-up row stands for: one video, or a playlist's entries. */
export function itemsFor(result: LookupResult | null): BatchItem[] {
  if (!result) return [];
  if (result.kind === "playlist") {
    return result.entries.map((e) => ({
      url: e.url,
      playlistIndex: e.playlistIndex,
      title: e.title,
      thumbnail: e.thumbnail,
      uploader: e.uploader,
      duration: e.duration,
    }));
  }
  return [{ url: result.url, title: result.title, thumbnail: result.thumbnail, uploader: result.uploader, duration: result.duration }];
}

/** The queue's identity for an item (downloader.rs enqueue: link and playlist position). */
export function itemKey(item: Pick<BatchItem, "url" | "playlistIndex">): string {
  const index = item.playlistIndex && item.playlistIndex > 0 ? item.playlistIndex : "";
  return `${index}
${item.url}`;
}

/** The row's items the queue has not accepted yet. */
export function remainingItems(row: BatchRow): BatchItem[] {
  const items = itemsFor(row.result);
  return row.accepted.size === 0 ? items : items.filter((item) => !row.accepted.has(itemKey(item)));
}

export function selectable(row: BatchRow): boolean {
  return row.status === "ready" && !row.added && remainingItems(row).length > 0;
}

export interface BatchCounts {
  waiting: number;
  looking: number;
  ready: number;
  failed: number;
  cancelled: number;
  added: number;
  selectedRows: number;
  /** Queue items the selected rows still have to add (playlists count their entries). */
  selectedItems: number;
}

export interface EnqueueSummary {
  added: number;
  /**
   * Items sent that the queue skipped: already queued with the same options, or repeated.
   * Includes a whole call refused with ALREADY_QUEUED.
   */
  skipped: number;
  failed: number;
  rowsAdded: number;
  rowsFailed: number;
  errors: string[];
}

export interface Chunk {
  items: BatchItem[];
  rowIds: number[];
  /** The row each item came from, in the same order as `items`. */
  owners: number[];
}

/**
 * Packs rows into enqueue calls of at most `max` items, keeping each row in one call
 * unless the row alone is larger than that (then it fills whole calls of its own).
 */
export function planChunks(rows: { id: number; items: BatchItem[] }[], max = MAX_ENQUEUE): Chunk[] {
  const chunks: Chunk[] = [];
  let current: Chunk = { items: [], rowIds: [], owners: [] };
  const close = () => {
    if (current.items.length > 0) chunks.push(current);
    current = { items: [], rowIds: [], owners: [] };
  };
  for (const row of rows) {
    if (row.items.length === 0) continue;
    // A row that does not fit starts a new call, so one refusal touches as few rows as possible.
    if (current.items.length + row.items.length > max) close();
    let rest = row.items;
    while (rest.length > 0) {
      const room = max - current.items.length;
      if (room === 0) {
        close();
        continue;
      }
      const part = rest.slice(0, room);
      current.items.push(...part);
      for (let i = 0; i < part.length; i++) current.owners.push(row.id);
      current.rowIds.push(row.id);
      rest = rest.slice(room);
    }
  }
  close();
  return chunks;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function describeEnqueue(summary: EnqueueSummary): string {
  const parts: string[] = [];
  if (summary.added > 0) parts.push(`Added ${plural(summary.added, "video", "videos")} to the queue.`);
  if (summary.skipped > 0) parts.push(`${summary.skipped} ${summary.skipped === 1 ? "was" : "were"} already queued or listed twice.`);
  if (summary.failed > 0) {
    const what = summary.added > 0 ? String(summary.failed) : plural(summary.failed, "video", "videos");
    parts.push(`${what} could not be added: ${summary.errors.join(" ")}`);
  }
  return parts.join(" ") || "Nothing was added.";
}

function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}

export interface LinkBatchOptions<R extends LookupResult> {
  /** Called whenever one row changes (not for a new or cleared batch). */
  onRow?: (row: BatchRow<R>) => void;
  /** Lookups at once; LOOKUP_CONCURRENCY by default. */
  limit?: number;
  /** Turns a rejected lookup or enqueue into a message. */
  describe?: (error: unknown) => string;
}

let nextRowId = 1;

/**
 * Looks up a list of links at most `limit` at a time. Each submission replaces the last;
 * a reply from an earlier submission, or for a row cancelled or retried since, is dropped.
 * A lookup already running cannot be stopped (the backend has no cancel), so it still
 * counts towards the limit until it answers.
 */
export class LinkBatch<R extends LookupResult = LookupResult> {
  rows: BatchRow<R>[] = [];
  /** Backend lookups running, including ones whose reply will be ignored. */
  inFlight = 0;
  submitting = false;
  private generation = 0;
  private queue: BatchRow<R>[] = [];
  private readonly onRow: (row: BatchRow<R>) => void;
  private readonly limit: number;
  private readonly describe: (error: unknown) => string;

  constructor(
    private readonly probe: (url: string) => Promise<R>,
    options: LinkBatchOptions<R> = {},
  ) {
    this.onRow = options.onRow ?? (() => undefined);
    this.limit = options.limit ?? LOOKUP_CONCURRENCY;
    this.describe = options.describe ?? describeError;
  }

  /** Replaces the batch with these links and starts looking them up. */
  start(links: ParsedLink[]) {
    this.drop();
    this.rows = links.map((link) => ({
      id: nextRowId++,
      url: link.url,
      line: link.line,
      status: "waiting",
      result: null,
      error: null,
      selected: false,
      added: false,
      addError: null,
      accepted: new Set(),
      token: 0,
    }));
    this.queue = [...this.rows];
    this.pump();
  }

  /** Forgets the batch; replies still on their way are ignored. */
  clear() {
    this.drop();
    this.rows = [];
  }

  /** Stops every lookup not yet answered. Rows can be looked up again with `retry`. */
  cancel() {
    this.queue = [];
    for (const row of this.rows) {
      if (row.status === "waiting" || row.status === "looking") this.markCancelled(row);
    }
  }

  cancelRow(id: number) {
    const row = this.find(id);
    if (!row || (row.status !== "waiting" && row.status !== "looking")) return;
    this.queue = this.queue.filter((r) => r !== row);
    this.markCancelled(row);
  }

  retry(id: number) {
    const row = this.find(id);
    if (!row || (row.status !== "failed" && row.status !== "cancelled")) return;
    this.enqueueLookup(row);
    this.pump();
  }

  retryAll() {
    for (const row of this.rows) {
      if (row.status === "failed" || row.status === "cancelled") this.enqueueLookup(row);
    }
    this.pump();
  }

  setSelected(id: number, selected: boolean) {
    const row = this.find(id);
    if (!row) return;
    const next = selected && selectable(row);
    if (row.selected === next) return;
    row.selected = next;
    this.onRow(row);
  }

  selectAll(selected: boolean) {
    for (const row of this.rows) this.setSelected(row.id, selected);
  }

  counts(): BatchCounts {
    const counts: BatchCounts = { waiting: 0, looking: 0, ready: 0, failed: 0, cancelled: 0, added: 0, selectedRows: 0, selectedItems: 0 };
    for (const row of this.rows) {
      counts[row.status] += 1;
      if (row.added) counts.added += 1;
      if (row.selected) {
        counts.selectedRows += 1;
        counts.selectedItems += remainingItems(row).length;
      }
    }
    return counts;
  }

  /**
   * Sends the selected rows to the queue through `send` (the existing enqueue command),
   * in calls of at most MAX_ENQUEUE items, one after another. Only items the queue has not
   * accepted are sent. A call refused with ALREADY_QUEUED counts as skipped: those items
   * are in the queue. Returns null when there is nothing to send or a submission is
   * already running.
   */
  async enqueue(send: (items: BatchItem[]) => Promise<{ jobs: unknown[] }>, max = MAX_ENQUEUE): Promise<EnqueueSummary | null> {
    if (this.submitting) return null;
    const rows = this.rows.filter((row) => row.selected && selectable(row));
    if (rows.length === 0) return null;
    this.submitting = true;
    const summary: EnqueueSummary = { added: 0, skipped: 0, failed: 0, rowsAdded: 0, rowsFailed: 0, errors: [] };
    const refused = new Map<number, string>();
    const byId = new Map(rows.map((row) => [row.id, row]));
    const accept = (chunk: Chunk) => chunk.items.forEach((item, i) => byId.get(chunk.owners[i])?.accepted.add(itemKey(item)));
    try {
      const chunks = planChunks(rows.map((row) => ({ id: row.id, items: remainingItems(row) })), max);
      for (const chunk of chunks) {
        try {
          const reply = await send(chunk.items);
          summary.added += reply.jobs.length;
          summary.skipped += Math.max(0, chunk.items.length - reply.jobs.length);
          accept(chunk);
        } catch (error) {
          const message = this.describe(error);
          if (message === ALREADY_QUEUED) {
            summary.skipped += chunk.items.length;
            accept(chunk);
            continue;
          }
          summary.failed += chunk.items.length;
          if (!summary.errors.includes(message)) summary.errors.push(message);
          for (const id of chunk.rowIds) if (!refused.has(id)) refused.set(id, message);
        }
      }
    } finally {
      this.submitting = false;
    }
    for (const row of rows) {
      if (remainingItems(row).length === 0) {
        row.added = true;
        row.selected = false;
        row.addError = null;
        summary.rowsAdded += 1;
      } else {
        row.addError = refused.get(row.id) ?? "Not every video was added.";
        summary.rowsFailed += 1;
      }
      this.onRow(row);
    }
    return summary;
  }

  private find(id: number) {
    return this.rows.find((row) => row.id === id);
  }

  private drop() {
    this.generation += 1;
    this.queue = [];
    for (const row of this.rows) {
      if (row.status === "waiting" || row.status === "looking") {
        row.status = "cancelled";
        row.token += 1;
      }
    }
  }

  private markCancelled(row: BatchRow<R>) {
    row.status = "cancelled";
    row.token += 1;
    this.onRow(row);
  }

  private enqueueLookup(row: BatchRow<R>) {
    row.status = "waiting";
    row.error = null;
    row.token += 1;
    this.queue.push(row);
    this.onRow(row);
  }

  private pump() {
    while (this.inFlight < this.limit && this.queue.length > 0) {
      const row = this.queue.shift()!;
      if (row.status === "waiting") this.launch(row);
    }
  }

  private launch(row: BatchRow<R>) {
    row.status = "looking";
    row.token += 1;
    const token = row.token;
    const generation = this.generation;
    this.inFlight += 1;
    this.onRow(row);
    let reply: Promise<R>;
    try {
      reply = Promise.resolve(this.probe(row.url));
    } catch (error) {
      reply = Promise.reject(error);
    }
    const current = () => generation === this.generation && row.token === token && row.status === "looking";
    reply.then(
      (result) => {
        this.inFlight -= 1;
        if (current()) {
          row.status = "ready";
          row.result = result;
          row.error = null;
          row.selected = selectable(row);
          this.onRow(row);
        }
        this.pump();
      },
      (error: unknown) => {
        this.inFlight -= 1;
        if (current()) {
          row.status = "failed";
          row.result = null;
          row.error = this.describe(error);
          row.selected = false;
          this.onRow(row);
        }
        this.pump();
      },
    );
  }
}
