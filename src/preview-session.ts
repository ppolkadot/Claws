// What the Download view keeps for a single link while another view is open: the details that
// were looked up, the choices made over them, and the estimates already read. Plain data only,
// bounded, and owned by the store (store.ts), so it outlives the view that made it. No DOM, and
// no imports beyond types, so the Node tests can run it (scripts/tests/preview-session.test.mjs).
//
// A running lookup or estimate is never kept: the view cancels those when it is left, and what
// they would have said is dropped, not revived. Only a finished run (EstimateRun.snapshot) and
// finished estimates are saved.

import { ESTIMATE_CAP, estimateState, type DownloadEstimate, type RunSnapshot } from "./download-estimates.js";

/** Why a check for changes ended unfinished: leaving the page, not a failure of the check. */
export const CHECK_INTERRUPTED = "The check for changes was interrupted when you left this page.";

/** The view as it was left. `R` is the lookup, `C` its choices, `I` a list's estimate item. */
export interface SavedPreview<R = unknown, C = unknown, I = unknown> {
  /** The link that was looked up; a save for any other link is not this session's. */
  url: string;
  /** The link as it was typed, which can differ from `url`; the link cache is keyed by both. */
  typedUrl: string;
  result: R;
  choices: C;
  /** The playlist entries ticked, by position in the list. */
  selected: number[];
  /** A check for changes still running when the view was left is saved as stale. */
  freshness: "fresh" | "stale";
  staleReason: string;
  /** What a refresh said it could not keep of the choices made. */
  notices: string[];
  /** A playlist's estimate run, when one finished or was stopped. */
  list: RunSnapshot<I> | null;
}

interface Epoch<R, C, I> {
  url: string;
  /** Moves on when a refresh changes what sizes depend on; estimates of older ones are dropped. */
  generation: number;
  /** Completed estimates by their key (which carries the link, position, options and generation). */
  estimates: Map<string, DownloadEstimate>;
  saved: SavedPreview<R, C, I> | null;
}

/**
 * One session at a time, for the link now being worked on. It starts with that link's lookup
 * (`begin`) and ends when another link or a batch replaces it, when the link is added to the
 * queue (`clear`), or when the view is left on something that is not a link's details.
 */
export class PreviewMemory<R = unknown, C = unknown, I = unknown> {
  private epoch: Epoch<R, C, I> | null = null;
  /** Links on their way to the queue; one of them is not shown as new to a view that returns. */
  private readonly addingUrls = new Set<string>();

  get active(): boolean {
    return this.epoch !== null;
  }

  get generation(): number {
    return this.epoch?.generation ?? 0;
  }

  /** Completed estimates held. */
  get size(): number {
    return this.epoch?.estimates.size ?? 0;
  }

  /** A new link's details replace any session, with its choices and estimates. */
  begin(url: string) {
    this.epoch = { url, generation: 0, estimates: new Map(), saved: null };
  }

  clear() {
    this.epoch = null;
  }

  /**
   * Ends the session only while it is still this link's: another link may have been looked up
   * while this one was being added, and its session is not done with.
   */
  clearFor(url: string) {
    this.addingUrls.delete(url);
    if (this.epoch?.url === url) this.epoch = null;
  }

  /**
   * The link is on its way to the queue. A view left now and returned to before the add ends
   * is not brought back, since the link is no longer new; `clearFor` ends the add.
   */
  adding(url: string) {
    this.addingUrls.add(url);
  }

  /** The add did not happen: the link is still just looked up. */
  addFailed(url: string) {
    this.addingUrls.delete(url);
  }

  /** The lookup settled on another address for the same link: the session follows it. */
  rekey(url: string) {
    if (this.epoch) this.epoch.url = url;
  }

  /** A refresh changed what sizes depend on: estimates made for the old details do not apply. */
  refreshed() {
    if (!this.epoch) return;
    this.epoch.generation += 1;
    this.epoch.estimates.clear();
  }

  /**
   * Keeps a copy of the view as it is now. Only plain data is kept: a value that cannot be
   * copied (a callback, a node) ends the session instead of being half kept.
   */
  save(saved: SavedPreview<R, C, I>) {
    if (!this.epoch || saved.url !== this.epoch.url) return;
    try {
      const copy = structuredClone(saved);
      if (copy.list) copy.list.settled = copy.list.settled.slice(0, ESTIMATE_CAP);
      this.epoch.saved = copy;
    } catch {
      this.epoch = null;
    }
  }

  /** The saved view, once: a later return does not revive it again. */
  take(): SavedPreview<R, C, I> | null {
    if (!this.epoch) return null;
    const saved = this.epoch.saved;
    this.epoch.saved = null;
    return saved && !this.addingUrls.has(saved.url) ? saved : null;
  }

  /** Keeps a completed estimate that has a size. Failed or unknown ones are asked again. */
  remember(key: string, estimate: DownloadEstimate) {
    const epoch = this.epoch;
    if (!epoch || estimateState(estimate) !== "available") return;
    epoch.estimates.delete(key);
    epoch.estimates.set(key, structuredClone(estimate));
    while (epoch.estimates.size > ESTIMATE_CAP) epoch.estimates.delete(epoch.estimates.keys().next().value!);
  }

  recall(key: string): DownloadEstimate | undefined {
    const estimates = this.epoch?.estimates;
    const found = estimates?.get(key);
    if (found && estimates) {
      estimates.delete(key); // used again: the least recently used goes first
      estimates.set(key, found);
    }
    return found;
  }
}

/** What an estimate depends on in a lookup: the sizes offered and which videos there are. */
interface SizeSource {
  heights: number[];
  resolutions?: unknown[] | null;
  /** Sizes are read from bitrate times length, so a changed length changes them. */
  duration?: number | null;
  entries: { url: string; playlistIndex: number | null; duration?: number | null }[];
}

const sizeShape = (r: SizeSource) =>
  JSON.stringify([r.heights, r.resolutions ?? null, r.duration ?? null, r.entries.map((e) => [e.url, e.playlistIndex, e.duration ?? null])]);

/** False when a refresh changed what estimates were read for (sizes offered, the videos, their length). */
export function estimatesStillApply(before: SizeSource | null | undefined, next: SizeSource): boolean {
  return before != null && sizeShape(before) === sizeShape(next);
}
