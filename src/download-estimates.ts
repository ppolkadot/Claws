// The numbers come from the backend (estimate.rs), which re-runs the download's own format
// selection over the cached lookup. They are display only: the download still checks the
// actual free space when it starts. A size that is not known is never counted as zero.
//
// An estimate is available (a size), unknown (the site gives none) or failed (the tool did not
// run, timed out or said nothing usable). A failure can be retried by the user, only for the
// items that failed and through the same shared call limit; nothing is retried automatically.

/** Backend calls at once when estimating many items; each one is a yt-dlp process. */
export const ESTIMATE_CONCURRENCY = 3;
/** Items one "Estimate Sizes" press covers; the rest are named as not estimated. */
export const ESTIMATE_CAP = 200;
/** A single link's preview waits this long for its choices to settle. */
export const ESTIMATE_DEBOUNCE_MS = 250;

export type Confidence = "exact" | "approximate" | "unknown";

/**
 * Whether a figure could be had: available (a size), unknown (the site gives none) or failed
 * (the estimate could not be made, so trying again may work). Older responses lack it.
 */
export type EstimateState = "available" | "unknown" | "failed";

export interface EstimatePart {
  formatId: string;
  vcodec: string | null;
  acodec: string | null;
  ext: string | null;
  width: number | null;
  height: number | null;
  bytes: number | null;
  exact: boolean;
}

export interface DownloadEstimate {
  /** Bytes transferred, or null when any selected stream's size is unknown. */
  downloadBytes: number | null;
  confidence: Confidence;
  parts: EstimatePart[];
  /** The saved file's container, when known before saving. */
  container: string | null;
  /** Peak bytes the work folder reaches while the download runs, before the safety margin. */
  peakTempBytes: number | null;
  /**
   * Free space the download asks for when it starts: the peak plus admission's safety
   * margin (disk_budget.rs `required`), without other downloads' reservations. Shown.
   */
  requiredFreeBytes: number | null;
  outputBytes: number | null;
  /** The kept file is a conversion (an audio target): its size and container are not the download's. */
  converted: boolean;
  unknownReason: string | null;
  /** Absent in older-shaped responses: read it through `estimateState`. */
  state?: EstimateState;
  /** For a failed estimate, a short single-line reason that is safe to show. */
  diagnostic?: string | null;
}

/**
 * The state of an estimate, whatever shape it arrived in. Only a response that says so is
 * failed; an older one is available when it has a size and unknown when it has none. A failure
 * has no size, and a state that says available without one is not a size either.
 */
export function estimateState(estimate: DownloadEstimate | null | undefined): EstimateState {
  if (estimate?.state === "failed") return "failed";
  return known(estimate?.downloadBytes) !== null ? "available" : "unknown";
}

const FAILURE_LIMIT = 240;
const NO_REASON = "The estimate could not be made.";

/**
 * A word that names a place on the machine: a `file:` link, a drive, rooted or backslashed
 * path, a drive letter inside a word (`--cookies=C:/x`), or a relative path with two or more
 * slashes. A web link is not one.
 */
function isPath(word: string): boolean {
  const bare = word.replace(/^["'(\[<]+|["')\]>,;]+$/g, "");
  const rooted = (w: string) => /^\/./.test(w);
  const eq = bare.indexOf("=");
  return (
    /(^|[^A-Za-z0-9])[A-Za-z]:(\\|\/(?!\/))/.test(bare) ||
    bare.includes("\\") ||
    /^file:/i.test(bare) ||
    rooted(bare) ||
    (eq >= 0 && rooted(bare.slice(eq + 1))) ||
    (!bare.includes("://") && (bare.match(/\//g)?.length ?? 0) >= 2)
  );
}

/** A rejected call as one short line that is safe to show: no local paths, bounded. */
export function describeFailure(error: unknown): string {
  const raw = typeof error === "string" ? error : error instanceof Error ? error.message : "";
  const line = raw
    .split(/\s+/)
    .filter(Boolean)
    .map((word) => (isPath(word) ? "<path>" : word))
    .join(" ");
  if (!line) return NO_REASON;
  return line.length > FAILURE_LIMIT ? `${line.slice(0, FAILURE_LIMIT)}\u2026` : line;
}

export function failedEstimate(diagnostic: string): DownloadEstimate {
  return {
    downloadBytes: null,
    confidence: "unknown",
    parts: [],
    container: null,
    peakTempBytes: null,
    requiredFreeBytes: null,
    outputBytes: null,
    converted: false,
    unknownReason: diagnostic,
    state: "failed",
    diagnostic,
  };
}

/** A usable size: positive and finite. Anything else is unknown, never zero. */
function known(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Binary units with the backend's labels (downloader.rs human_bytes): "812 MB", "2.1 GB". */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const round = (v: number) => (unit === 0 || v >= 100 ? Math.round(v) : Math.round(v * 10) / 10);
  // 1023.96 MB shows as 1.0 GB, not 1024 MB.
  if (round(value) >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const shown = round(value);
  const text = unit === 0 || shown >= 100 ? shown.toLocaleString("en-US") : shown.toFixed(1);
  return `${text} ${UNITS[unit]}`;
}

const items = (n: number) => `${n} ${n === 1 ? "item" : "items"}`;

export interface Aggregate {
  items: number;
  knownCount: number;
  /** Items the site gives no size for (not those whose estimate failed). */
  unknownCount: number;
  /** Items whose estimate could not be made, worth retrying; never counted as sizes. */
  failedCount: number;
  /** Sum of the known sizes; null when none is known. */
  knownBytes: number | null;
  /** Some known size is an estimate. */
  approximate: boolean;
  /** The largest free space one item asks for (peak plus margin), since each download checks its own. */
  requiredFreeBytes: number | null;
  peakUnknownCount: number;
  /** Some item is converted, so the kept files are sized apart from the download. */
  converted: boolean;
  /** Sum of the known kept-file sizes of converted items; null when none is known. */
  outputBytes: number | null;
  outputUnknownCount: number;
  text: string;
  /** The kept files of a conversion ("Saved as WAV: about 1.1 GB"); null when nothing is converted. */
  outputText: string | null;
  /** The free-space line, shown apart from the size; null when there is nothing to say. */
  tempText: string | null;
}

const unknownItems = (n: number) => (n === 1 ? "1 item has an unknown size" : `${n} items have unknown sizes`);
const failedItems = (n: number) => (n === 1 ? "the estimate failed for 1 item" : `the estimates failed for ${n} items`);

/**
 * Totals for items estimated. A failed estimate is counted apart from an item with no size,
 * and neither adds anything to the total. (null is an older "no answer", counted as unknown.)
 */
export function aggregate(estimates: (DownloadEstimate | null)[]): Aggregate {
  let knownCount = 0;
  let failedCount = 0;
  let knownBytes = 0;
  let approximate = false;
  let need: number | null = null;
  let peakUnknownCount = 0;
  let converted = false;
  let container: string | null = null;
  let outputKnownCount = 0;
  let outputBytes = 0;
  for (const estimate of estimates) {
    const failed = estimateState(estimate) === "failed";
    if (failed) failedCount += 1;
    const bytes = failed ? null : known(estimate?.downloadBytes);
    if (bytes !== null) {
      knownCount += 1;
      knownBytes += bytes;
      if (estimate!.confidence !== "exact") approximate = true;
    }
    const itemNeed = bytes === null ? null : known(estimate?.requiredFreeBytes);
    if (itemNeed === null) peakUnknownCount += 1;
    else need = Math.max(need ?? 0, itemNeed);
    if (estimate?.converted) {
      converted = true;
      container ??= estimate.container;
      const output = bytes === null ? null : known(estimate.outputBytes);
      if (output !== null) {
        outputKnownCount += 1;
        outputBytes += output;
      }
    }
  }
  const count = estimates.length;
  const unknownCount = count - knownCount - failedCount;
  const missing = [unknownCount > 0 ? unknownItems(unknownCount) : "", failedCount > 0 ? failedItems(failedCount) : ""].filter(Boolean).join("; ");

  let text: string;
  if (count === 0) text = "";
  else if (knownCount === 0) {
    if (failedCount === 0) text = count === 1 ? "Size unknown" : `Sizes unknown for all ${count} items`;
    else if (unknownCount === 0) text = count === 1 ? "Size estimate failed" : `Size estimates failed for all ${count} items`;
    else text = missing.charAt(0).toUpperCase() + missing.slice(1);
  } else {
    const size = formatSize(knownBytes);
    if (missing === "") text = `${approximate ? "About" : "Exactly"} ${size}`;
    // An exact total covers only the items it counts; say which.
    else text = approximate ? `About ${size}; ${missing}` : `Exactly ${size} for ${items(knownCount)}; ${missing}`;
  }

  // One set of choices converts every item alike; an item whose estimate failed has an
  // unknown saved size too.
  const outputUnknownCount = converted ? count - outputKnownCount : 0;
  let outputText: string | null = null;
  if (converted) {
    const saved = `Saved as ${container ? container.toUpperCase() : "a converted file"}`;
    if (outputKnownCount === 0) outputText = `${saved}; ${count === 1 ? "saved size unknown" : "saved sizes unknown"}`;
    else if (outputUnknownCount === 0) outputText = `${saved}: about ${formatSize(outputBytes)}`;
    else {
      const rest = outputUnknownCount === 1 ? "1 item has an unknown saved size" : `${outputUnknownCount} items have unknown saved sizes`;
      outputText = `${saved}: about ${formatSize(outputBytes)} for ${items(outputKnownCount)}; ${rest}`;
    }
  }

  // The figure admission asks for (the peak plus its safety margin), so a download this
  // line says fits is not then refused for want of the margin.
  let tempText: string | null = null;
  if (knownCount > 0) {
    if (need === null) tempText = "Free space needed while downloading is unknown";
    else if (count === 1) tempText = `Needs about ${formatSize(need)} free while downloading, including a safety margin`;
    else {
      tempText = `The largest item needs about ${formatSize(need)} free while downloading, including a safety margin`;
      if (peakUnknownCount > 0) tempText += `; unknown for ${items(peakUnknownCount)}`;
    }
  }

  return {
    items: count,
    knownCount,
    unknownCount,
    failedCount,
    knownBytes: knownCount > 0 ? knownBytes : null,
    approximate,
    requiredFreeBytes: need,
    peakUnknownCount,
    converted,
    outputBytes: outputKnownCount > 0 ? outputBytes : null,
    outputUnknownCount,
    text,
    outputText,
    tempText,
  };
}


export type PreviewStatus = "idle" | "loading" | "done" | "failed";

export interface PreviewState {
  status: PreviewStatus;
  estimate?: DownloadEstimate;
  /** Why a failed preview failed: one short line that is safe to show. */
  diagnostic?: string;
}

const FAILED_SIZE = "Size estimate failed";

export function describeSingle(state: PreviewState): { size: string; detail: string } {
  switch (state.status) {
    case "idle":
      return { size: "", detail: "" };
    case "loading":
      return { size: "Estimating size…", detail: "" };
    case "failed":
      return { size: FAILED_SIZE, detail: state.diagnostic ?? "" };
    case "done": {
      const estimate = state.estimate!;
      if (estimateState(estimate) === "failed") return { size: FAILED_SIZE, detail: estimate.diagnostic ?? estimate.unknownReason ?? "" };
      const sum = aggregate([estimate]);
      if (sum.knownCount === 0) return { size: sum.text, detail: estimate.unknownReason ?? "" };
      if (sum.converted) {
        // The download and the kept file differ (a 12 MB Opus stream saved as a 115 MB
        // WAV), so each is named for what it is.
        const downloads = `Downloads ${sum.text.charAt(0).toLowerCase()}${sum.text.slice(1)}`;
        const target = estimate.container ? estimate.container.toUpperCase() : "a converted file";
        const kept = sum.outputBytes === null ? "size unknown" : `about ${formatSize(sum.outputBytes)}`;
        return { size: `${downloads}  ·  saved as ${target}, ${kept}`, detail: sum.tempText ?? "" };
      }
      const video = estimate.parts.find((p) => p.vcodec && p.width && p.height);
      const what = [estimate.container?.toUpperCase(), video ? `${video.width}×${video.height}` : null].filter(Boolean).join(", ");
      return { size: what ? `${sum.text}  ·  ${what}` : sum.text, detail: sum.tempText ?? "" };
    }
  }
}

/**
 * A limit on estimate calls running at once, shared by everything that asks (a list's run,
 * the batch's run, a single link's preview), so a run that was reset or stopped still counts
 * until its calls answer: a call already sent is a yt-dlp process that cannot be stopped from
 * here. A caller that finds no free slot waits; `give` wakes waiters while slots are free.
 */
export class CallLimit {
  private running = 0;
  private readonly sleepers = new Set<() => void>();

  constructor(readonly limit: number = ESTIMATE_CONCURRENCY) {}

  get free(): boolean {
    return this.running < this.limit;
  }

  get active(): number {
    return this.running;
  }

  take() {
    this.running += 1;
  }

  give() {
    this.running = Math.max(0, this.running - 1);
    for (const wake of [...this.sleepers]) {
      if (!this.free) break;
      this.sleepers.delete(wake);
      wake();
    }
  }

  /** Calls `wake` once a slot is free (it takes the slot itself, or waits again). */
  wait(wake: () => void) {
    this.sleepers.add(wake);
  }

  leave(wake: () => void) {
    this.sleepers.delete(wake);
  }
}

export interface Timers {
  set(fn: () => void, ms: number): unknown;
  clear(id: unknown): void;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (id) => clearTimeout(id as ReturnType<typeof setTimeout>),
};

/**
 * One link's preview. Each change of choices is a new key; the estimate is asked for once
 * the choices have settled for ESTIMATE_DEBOUNCE_MS, and only the latest key's answer is
 * kept. A failure (a call that threw, or an answer that says it failed) is a failed preview,
 * not an error and not an unknown size; `retry` asks again once, when the user asks.
 *
 * At most one backend call runs at a time (each is a yt-dlp process that cannot be
 * stopped from here): a settled key that arrives while one runs waits for it, and only
 * the latest waiting key is asked next. With a shared `calls` limit it also waits while
 * other estimates fill every slot.
 */
export class PreviewEstimator {
  state: PreviewState = { status: "idle" };
  private key: string | null = null;
  private ticket = 0;
  private timer: unknown = null;
  private inFlight = false;
  private waiting: (() => void) | null = null;
  private readonly timers: Timers;
  private readonly delay: number;
  private readonly calls: CallLimit;
  private readonly onChange: () => void;

  constructor(
    private readonly estimate: (key: string) => Promise<DownloadEstimate>,
    options: { timers?: Timers; delay?: number; calls?: CallLimit; onChange?: () => void } = {},
  ) {
    this.timers = options.timers ?? realTimers;
    this.delay = options.delay ?? ESTIMATE_DEBOUNCE_MS;
    this.calls = options.calls ?? new CallLimit();
    this.onChange = options.onChange ?? (() => undefined);
  }

  /** Starts the waiting key once this preview is idle and a shared slot is free. */
  private readonly pump = () => {
    if (this.inFlight || this.waiting === null) return this.calls.leave(this.pump);
    if (!this.calls.free) return this.calls.wait(this.pump);
    this.calls.leave(this.pump);
    const next = this.waiting;
    this.waiting = null;
    next();
  };

  request(key: string) {
    if (key === this.key && this.state.status !== "idle") return;
    this.stop();
    this.key = key;
    const mine = this.ticket;
    this.set({ status: "loading" });
    const launch = () => {
      this.inFlight = true;
      this.calls.take();
      let reply: Promise<DownloadEstimate>;
      try {
        reply = Promise.resolve(this.estimate(key));
      } catch (error) {
        reply = Promise.reject(error);
      }
      const settle = (state: PreviewState) => {
        this.inFlight = false;
        if (mine === this.ticket) this.set(state);
        this.calls.give();
        this.pump();
      };
      reply.then(
        (estimate) =>
          settle(
            estimateState(estimate) === "failed"
              ? { status: "failed", diagnostic: describeFailure(estimate.diagnostic ?? estimate.unknownReason) }
              : { status: "done", estimate },
          ),
        (error) => settle({ status: "failed", diagnostic: describeFailure(error) }),
      );
    };
    this.timer = this.timers.set(() => {
      this.timer = null;
      this.waiting = launch;
      this.pump();
    }, this.delay);
  }

  /** The choices the preview shows or is asking for; null when it shows nothing. */
  get currentKey(): string | null {
    return this.key;
  }

  /**
   * Shows an estimate already made for these choices, asking nothing. An answer still on its
   * way for other choices is ignored.
   */
  restore(key: string, estimate: DownloadEstimate) {
    this.stop();
    this.key = key;
    this.set({ status: "done", estimate });
  }

  get failed(): boolean {
    return this.state.status === "failed";
  }

  /**
   * Asks again for the choices whose estimate failed, once per call: a failure is never
   * retried by itself. It goes through the same debounce and shared call limit as a request.
   */
  retry() {
    if (this.state.status !== "failed" || this.key === null) return;
    const key = this.key;
    this.key = null;
    this.request(key);
  }

  /** Forgets the preview; an answer still on its way is ignored. */
  cancel() {
    this.stop();
    this.key = null;
    this.set({ status: "idle" });
  }

  private stop() {
    this.ticket += 1;
    if (this.timer !== null) this.timers.clear(this.timer);
    this.timer = null;
    this.waiting = null; // a key that was waiting is superseded too
    this.calls.leave(this.pump);
  }

  private set(state: PreviewState) {
    this.state = state;
    this.onChange();
  }
}


export interface RunState {
  /** Items asked for, before the cap. */
  total: number;
  /** Items this run estimates (at most the cap). */
  requested: number;
  done: number;
  running: boolean;
  cancelled: boolean;
  /** Items asked for that were not estimated (beyond the cap, or after Stop). */
  notEstimated: number;
  /** Items selected after the run started; it does not cover them (see `follow`). */
  notCovered: number;
  /**
   * Finished estimates, a failed call being a failed estimate (estimateState), one for each
   * item and in the order the items first answered; a retry replaces an item's in place.
   */
  results: DownloadEstimate[];
  /** A retry of the failed items is running (`running` is true too). */
  retrying: boolean;
  retryTotal: number;
  retryDone: number;
}

/**
 * A finished run as plain data, so it can wait while another view is open and come back
 * (EstimateRun.snapshot / restore). It holds no calls and no callbacks.
 */
export interface RunSnapshot<T> {
  key: string;
  asked: string[];
  total: number;
  requested: number;
  done: number;
  cancelled: boolean;
  notEstimated: number;
  settled: { item: T; result: DownloadEstimate }[];
}

const emptyRun = (): RunState => ({
  total: 0,
  requested: 0,
  done: 0,
  running: false,
  cancelled: false,
  notEstimated: 0,
  notCovered: 0,
  results: [],
  retrying: false,
  retryTotal: 0,
  retryDone: 0,
});

/**
 * Estimates a list of items only when asked: the first `cap` distinct items, with at most
 * ESTIMATE_CONCURRENCY backend calls at once across every run sharing the `calls` limit.
 * Stop or a new run drops the rest; replies from an earlier run are ignored. A call already
 * running cannot be stopped, so it counts towards the shared limit until it answers.
 *
 * Items are told apart by `id` (the queue's identity for an item), and repeats are
 * dropped, since the queue adds each item once: the total is what a download would add.
 */
export class EstimateRun<T> {
  /** The choices the run was started with; null when there is none. */
  key: string | null = null;
  state: RunState = emptyRun();
  private generation = 0;
  private queue: T[] = [];
  private asked = new Set<string>();
  /** What each answered item came to, by id, so a retry can ask for the failures alone. */
  private settled = new Map<string, { item: T; result: DownloadEstimate }>();
  private readonly calls: CallLimit;
  private readonly cap: number;
  private readonly id: (item: T) => string;
  private readonly onChange: () => void;

  constructor(
    private readonly estimate: (item: T) => Promise<DownloadEstimate>,
    options: { limit?: number; calls?: CallLimit; cap?: number; id?: (item: T) => string; onChange?: () => void } = {},
  ) {
    this.calls = options.calls ?? new CallLimit(options.limit ?? ESTIMATE_CONCURRENCY);
    this.cap = options.cap ?? ESTIMATE_CAP;
    this.id = options.id ?? ((item) => JSON.stringify(item));
    this.onChange = options.onChange ?? (() => undefined);
  }

  get maxItems(): number {
    return this.cap;
  }

  distinct(list: T[]): T[] {
    const seen = new Set<string>();
    return list.filter((item) => {
      const id = this.id(item);
      if (seen.has(id)) return false;
      seen.add(id);
      return true;
    });
  }

  start(list: T[], key: string) {
    this.generation += 1;
    this.calls.leave(this.pump);
    const unique = this.distinct(list);
    this.key = key;
    this.asked = new Set(unique.map(this.id));
    this.settled = new Map();
    this.queue = unique.slice(0, this.cap);
    this.state = {
      ...emptyRun(),
      total: unique.length,
      requested: this.queue.length,
      running: this.queue.length > 0,
      notEstimated: unique.length - this.queue.length,
    };
    this.onChange();
    this.pump();
  }

  /**
   * Matches the run with the choices and selection now on screen. Other choices, or an
   * item the run was asked for that is no longer selected, end it (it is reset, which
   * repaints, and false is returned). Items selected since it started (a link that finished
   * reading, an entry ticked) do not: the run goes on and counts them in `notCovered`.
   */
  follow(key: string, list: T[]): boolean {
    if (this.key === null) return true;
    if (key !== this.key) {
      this.reset();
      return false;
    }
    const now = new Set(this.distinct(list).map(this.id));
    for (const id of this.asked) {
      if (!now.has(id)) {
        this.reset();
        return false;
      }
    }
    this.state.notCovered = now.size - this.asked.size;
    return true;
  }

  /**
   * Stops asking; what was estimated stays, the rest is named as not estimated. Stopping a
   * retry leaves the items that failed failed (and retryable): it is not a stopped run.
   */
  cancel() {
    if (!this.state.running) return;
    this.generation += 1;
    this.queue = [];
    this.calls.leave(this.pump);
    this.state.running = false;
    if (this.state.retrying) this.state.retrying = false;
    else {
      this.state.cancelled = true;
      this.state.notEstimated = this.state.total - this.state.done;
    }
    this.onChange();
  }

  private failures(): T[] {
    return [...this.settled.values()].filter((entry) => estimateState(entry.result) === "failed").map((entry) => entry.item);
  }

  get failedCount(): number {
    return this.failures().length;
  }

  get canRetry(): boolean {
    return this.key !== null && !this.state.running && this.failures().length > 0;
  }

  /**
   * Asks again for the items that failed, once each and only when called (never by itself),
   * through the shared call limit. Estimates that are fine are kept as they are. `key` is the
   * choices now on screen: with other choices the run no longer applies, so it is reset and
   * nothing is asked. False when nothing was started.
   */
  retryFailed(key: string): boolean {
    if (this.key === null || this.state.running) return false;
    if (key !== this.key) {
      this.reset();
      return false;
    }
    const failed = this.failures();
    if (failed.length === 0) return false;
    this.generation += 1; // a reply still on its way from before is not part of this pass
    this.queue = failed;
    this.state.running = true;
    this.state.retrying = true;
    this.state.retryTotal = failed.length;
    this.state.retryDone = 0;
    this.onChange();
    this.pump();
    return true;
  }

  /**
   * The finished run as plain data, or null when there is none to keep. A run still going has
   * no snapshot: its calls are not data. Stop it first (`cancel`) to keep what it has read.
   */
  snapshot(): RunSnapshot<T> | null {
    if (this.key === null || this.state.running) return null;
    const s = this.state;
    return {
      key: this.key,
      asked: [...this.asked],
      total: s.total,
      requested: s.requested,
      done: s.done,
      cancelled: s.cancelled,
      notEstimated: s.notEstimated,
      settled: [...this.settled.values()].map(({ item, result }) => ({ item, result })),
    };
  }

  /**
   * Takes a saved run back, as a run that is not going: nothing is asked and a failure in it can
   * still be retried by the user. Keeps the cap; a reply from any earlier run is ignored.
   */
  restore(snapshot: RunSnapshot<T>) {
    this.generation += 1;
    this.queue = [];
    this.calls.leave(this.pump);
    const settled = snapshot.settled.slice(0, this.cap);
    this.key = snapshot.key;
    this.asked = new Set(snapshot.asked);
    this.settled = new Map(settled.map((entry) => [this.id(entry.item), entry]));
    this.state = {
      ...emptyRun(),
      total: snapshot.total,
      requested: Math.min(snapshot.requested, this.cap),
      done: Math.min(snapshot.done, settled.length),
      cancelled: snapshot.cancelled,
      notEstimated: snapshot.notEstimated,
      results: settled.map((entry) => entry.result),
    };
    this.onChange();
  }

  /** Forgets the run (its items or choices changed). */
  reset() {
    this.generation += 1;
    this.queue = [];
    this.calls.leave(this.pump);
    this.key = null;
    this.asked = new Set();
    this.settled = new Map();
    this.state = emptyRun();
    this.onChange();
  }

  /** Starts queued items while the shared limit has room; otherwise waits for a slot. */
  private readonly pump = () => {
    while (this.calls.free && this.queue.length > 0) this.launch(this.queue.shift()!);
    if (this.queue.length > 0) this.calls.wait(this.pump);
    else this.calls.leave(this.pump);
  };

  private launch(item: T) {
    const generation = this.generation;
    const retry = this.state.retrying;
    this.calls.take();
    let reply: Promise<DownloadEstimate>;
    try {
      reply = Promise.resolve(this.estimate(item));
    } catch (error) {
      reply = Promise.reject(error);
    }
    const settle = (result: DownloadEstimate) => {
      if (generation === this.generation) {
        this.settled.set(this.id(item), { item, result });
        this.state.results = [...this.settled.values()].map((entry) => entry.result);
        if (retry) {
          this.state.retryDone += 1;
          if (this.state.retryDone === this.state.retryTotal) {
            this.state.running = false;
            this.state.retrying = false;
          }
        } else {
          this.state.done += 1;
          if (this.state.done === this.state.requested) this.state.running = false;
        }
        this.onChange();
      }
      this.calls.give(); // wakes whichever run or preview waits for a slot
      this.pump();
    };
    reply.then(
      (result) => settle(result && typeof result === "object" ? result : failedEstimate(NO_REASON)),
      (error) => settle(failedEstimate(describeFailure(error))),
    );
  }
}

export function describeRun<T>(run: EstimateRun<T>): string {
  const s = run.state;
  if (run.key === null || s.total === 0) return "";
  const later =
    s.notCovered > 0
      ? `${s.notCovered} ${s.notCovered === 1 ? "item" : "items"} selected after estimating began ${s.notCovered === 1 ? "is" : "are"} not included`
      : "";
  if (s.running && s.retrying) {
    return `Retrying ${s.retryTotal} failed ${s.retryTotal === 1 ? "estimate" : "estimates"}… ${s.retryDone} of ${s.retryTotal}${later ? `; ${later}` : ""}`;
  }
  if (s.running) return `Estimating sizes… ${s.done} of ${s.requested}${later ? `; ${later}` : ""}`;
  const sum = aggregate(s.results);
  const lines = [sum.text, sum.outputText ?? ""].filter(Boolean);
  if (s.cancelled) lines.push(`Stopped after ${s.done} of ${s.total}; ${s.notEstimated} ${s.notEstimated === 1 ? "was" : "were"} not estimated`);
  else if (s.notEstimated > 0) lines.push(`Only the first ${s.requested} selected items were estimated; ${s.notEstimated} ${s.notEstimated === 1 ? "was" : "were"} not`);
  if (later) lines.push(later);
  const sentence = lines.length > 1 || s.cancelled || s.notEstimated > 0;
  return sentence ? `${lines.join(". ")}.` : lines.join("");
}
