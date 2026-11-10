import type { ProbeResult, SpotifyLookupIssue, SpotifyResolution, SpotifyTrack, YouTubeCandidate } from "./api.js";

export type SpotifyMatchStage = "resolving" | "editable" | "searching" | "choosing" | "probing" | "confirmed" | "failed";

export interface SpotifyMatchSnapshot {
  stage: SpotifyMatchStage;
  /** The original Spotify input, never a media/cache URL. */
  url: string;
  track: SpotifyTrack | null;
  query: string;
  candidates: YouTubeCandidate[];
  selectedId: string | null;
  confirmed: ProbeResult | null;
  warnings: string[];
  error: string | null;
  /** Whether trying the same input again can help; false for input that can never work. */
  retryable: boolean;
}

type Immutable<T> = T extends object ? { readonly [K in keyof T]: Immutable<T[K]> } : T;
export type SpotifyMatchState = Immutable<SpotifyMatchSnapshot>;

export interface SpotifyMatchDependencies {
  resolve(url: string): Promise<SpotifyResolution>;
  search(track: SpotifyTrack, query: string): Promise<YouTubeCandidate[]>;
  probe(url: string): Promise<ProbeResult>;
}

/** Route recognizable Spotify inputs to their dedicated explanation, even unsupported entities. */
export function isSpotifyInput(raw: string): boolean {
  if (/^spotify:/i.test(raw.trim())) return true;
  try {
    const host = new URL(raw.trim()).hostname.toLowerCase();
    return host === "open.spotify.com" || host === "spotify.link";
  } catch { return false; }
}

export const MAX_QUERY_CODE_POINTS = 300;
const QUERY_SUFFIX = " audio";
const length = (text: string) => Array.from(text).length;

/**
 * The automatic search: `Artist, Artist - Title audio`, kept within the backend's 300
 * code points. The title and the ` audio` suffix are reserved first; whole artist names
 * fill what is left, in order. A title too long for the cap is cut at a word boundary
 * where possible. Only this derived query is shortened, never the Spotify metadata.
 */
export function initialSpotifyQuery(track: SpotifyTrack): string {
  const title = track.title?.trim();
  if (!title) return "";
  const budget = MAX_QUERY_CODE_POINTS - length(QUERY_SUFFIX);
  if (length(title) > budget) {
    const points = Array.from(title).slice(0, budget);
    let space = points.length - 1;
    while (space > 0 && !/\s/.test(points[space])) space -= 1;
    const cut = (space > 0 ? points.slice(0, space) : points).join("").trimEnd();
    return `${cut}${QUERY_SUFFIX}`;
  }
  const names: string[] = [];
  for (const artist of track.artists.map((name) => name.trim()).filter(Boolean)) {
    const next = [...names, artist].join(", ");
    if (length(next) + 3 + length(title) > budget) break;
    names.push(artist);
  }
  return `${names.length ? `${names.join(", ")} - ` : ""}${title}${QUERY_SUFFIX}`;
}

/** Normalizes whatever a failed lookup threw into a readable issue. */
export function toLookupIssue(error: unknown): SpotifyLookupIssue {
  if (error && typeof error === "object" && typeof (error as SpotifyLookupIssue).message === "string" && (error as SpotifyLookupIssue).message.trim()) {
    const issue = error as Partial<SpotifyLookupIssue>;
    return { code: typeof issue.code === "string" ? issue.code : "unknown", message: issue.message!, retryable: issue.retryable !== false };
  }
  const text = typeof error === "string" && error.trim() ? error : "Couldn't read this Spotify link.";
  return { code: "unknown", message: text, retryable: true };
}

function trackStub(raw: string): SpotifyTrack | null {
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== "https:" || parsed.hostname !== "open.spotify.com" || parsed.username || parsed.password || parsed.port) return null;
    const match = /^\/(?:intl-[A-Za-z-]+\/)?track\/([A-Za-z0-9]{22})\/?$/.exec(parsed.pathname);
    if (!match) return null;
    return { id: match[1], url: `https://open.spotify.com/track/${match[1]}`, title: null, artists: [], duration: null, artwork: null };
  } catch { return null; }
}

function youtubeId(raw: string): string | null {
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.port) return null;
    const id = parsed.hostname === "youtu.be" ? /^\/([\w-]{11})\/?$/.exec(parsed.pathname)?.[1]
      : (parsed.hostname === "www.youtube.com" || parsed.hostname === "youtube.com") && parsed.pathname === "/watch" ? parsed.searchParams.get("v") : null;
    return id && /^[\w-]{11}$/.test(id) ? id : null;
  } catch { return null; }
}

function freeze<T>(value: T): Immutable<T> {
  if (value && typeof value === "object") {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value as Immutable<T>;
}

const fresh = (url = ""): SpotifyMatchSnapshot => ({ stage: "editable", url, track: trackStub(url), query: "", candidates: [], selectedId: null, confirmed: null, warnings: [], error: null, retryable: false });
const metadataError = "Couldn't read this Spotify track. Enter the artist and song title to search YouTube.";
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/** One matching session. Every input change invalidates all work started against older state. */
export class SpotifyMatchController {
  private current: SpotifyMatchState = freeze(fresh());
  private epoch = 0;
  private disposed = false;
  private searchingQuery: string | null = null;

  constructor(private readonly dependencies: SpotifyMatchDependencies, private readonly onChange: (state: SpotifyMatchState) => void = () => {}) {}

  get state(): SpotifyMatchState { return this.current; }

  private publish(next: SpotifyMatchSnapshot): void {
    if (this.disposed) return;
    this.current = freeze(next);
    this.onChange(this.current);
  }

  private copy(): SpotifyMatchSnapshot { return structuredClone(this.current) as SpotifyMatchSnapshot; }
  private active(ticket: number): boolean { return !this.disposed && ticket === this.epoch; }

  async open(url: string): Promise<void> {
    if (this.disposed) return;
    const ticket = ++this.epoch;
    this.publish({ ...fresh(url), stage: "resolving" });
    try {
      const resolution = await this.dependencies.resolve(url);
      if (!this.active(ticket)) return;
      const next = fresh(url);
      next.track = structuredClone(resolution.track);
      next.warnings = [...resolution.warnings];
      next.query = initialSpotifyQuery(resolution.track);
      if (!next.query) next.error = metadataError;
      this.publish(next);
      // A callback can synchronously replace the input or edit the query.
      if (this.active(ticket) && next.query) await this.search();
    } catch (error) {
      if (!this.active(ticket)) return;
      const issue = toLookupIssue(error);
      const next = fresh(url);
      next.retryable = issue.retryable;
      next.stage = next.track ? "editable" : "failed";
      next.error = next.track ? metadataError : issue.retryable ? `${issue.message} Retry, or paste the full Spotify track link.` : issue.message;
      this.publish(next);
    }
  }

  setQuery(query: string): void {
    if (this.disposed || query === this.current.query) return;
    ++this.epoch;
    this.publish({ ...this.copy(), stage: "editable", query, candidates: [], selectedId: null, confirmed: null, error: null, retryable: this.current.track ? false : this.current.retryable });
  }

  async search(): Promise<void> {
    if (this.disposed) return;
    // The same query is already being searched: another request would only repeat the work.
    if (this.current.stage === "searching" && this.searchingQuery === this.current.query.trim()) return;
    const ticket = ++this.epoch;
    const next = this.copy();
    next.confirmed = null;
    next.selectedId = null;
    next.candidates = [];
    next.error = null;
    const query = next.query.trim();
    if (!next.track || !query || Array.from(query).length > 300) {
      next.stage = "editable";
      next.error = !next.track ? "Paste the full Spotify track link to search." : !query ? "Enter the artist and song title to search YouTube." : "Keep the search query to 300 characters or fewer.";
      this.publish(next);
      return;
    }
    next.stage = "searching";
    next.retryable = false;
    this.searchingQuery = query;
    this.publish(next);
    try {
      const candidates = await this.dependencies.search(structuredClone(next.track), query);
      if (!this.active(ticket)) return;
      const seen = new Set<string>();
      const valid = candidates.filter((candidate) => {
        if (youtubeId(candidate.url) !== candidate.videoId || seen.has(candidate.videoId)) return false;
        seen.add(candidate.videoId);
        return true;
      }).slice(0, 5).map((candidate) => ({ ...structuredClone(candidate), url: `https://www.youtube.com/watch?v=${candidate.videoId}` }));
      this.publish({ ...this.copy(), stage: "choosing", candidates: valid, error: null, retryable: false });
    } catch (error) {
      if (this.active(ticket)) this.publish({ ...this.copy(), stage: "editable", error: `Couldn't search YouTube. ${message(error)} Try again or edit the query.` });
    }
  }

  select(videoId: string): void {
    if (this.disposed || this.current.selectedId === videoId || !this.current.candidates.some((candidate) => candidate.videoId === videoId)) return;
    ++this.epoch;
    this.publish({ ...this.copy(), stage: "choosing", selectedId: videoId, confirmed: null, error: null });
  }

  async confirm(): Promise<void> {
    if (this.disposed || this.current.stage === "probing" || this.current.stage === "confirmed") return;
    const selected = this.current.candidates.find((candidate) => candidate.videoId === this.current.selectedId);
    if (!selected) return;
    const ticket = ++this.epoch;
    this.publish({ ...this.copy(), stage: "probing", confirmed: null, error: null });
    try {
      const result = await this.dependencies.probe(selected.url);
      if (!this.active(ticket)) return;
      if (result.kind !== "video" || youtubeId(result.url) !== selected.videoId) {
        this.publish({ ...this.copy(), stage: "choosing", error: "YouTube returned a different recording. Choose a match and try again." });
      } else if (result.audioAvailable !== true) {
        this.publish({ ...this.copy(), stage: "choosing", error: "Couldn't verify available audio for this recording. Retry or choose another match." });
      } else {
        this.publish({ ...this.copy(), stage: "confirmed", confirmed: { ...structuredClone(result), url: selected.url }, error: null });
      }
    } catch (error) {
      if (this.active(ticket)) this.publish({ ...this.copy(), stage: "choosing", error: `Couldn't check this recording. ${message(error)} Retry or choose another match.` });
    }
  }

  snapshot(): SpotifyMatchSnapshot {
    const saved = this.copy();
    if (saved.stage === "resolving" || saved.stage === "searching" || saved.stage === "probing") {
      const operation = saved.stage;
      saved.stage = saved.candidates.length ? "choosing" : saved.track ? "editable" : "failed";
      saved.confirmed = null;
      saved.retryable = operation === "resolving";
      saved.error = operation === "resolving"
        ? "This lookup was interrupted when you left this page. Retry to continue."
        : operation === "searching"
          ? "This search was interrupted when you left this page. Choose Find on YouTube to continue."
          : "This audio check was interrupted when you left this page. Choose Use this match to continue.";
    }
    return saved;
  }

  restore(snapshot: SpotifyMatchSnapshot): void {
    if (this.disposed) return;
    ++this.epoch;
    this.current = freeze(structuredClone(snapshot));
    const saved = this.snapshot();
    // Restore only internally consistent confirmations; it never starts a new lookup.
    if ((saved.stage === "confirmed" && !saved.confirmed) || (saved.confirmed && (saved.stage !== "confirmed" || saved.confirmed.kind !== "video" || saved.confirmed.audioAvailable !== true || youtubeId(saved.confirmed.url) !== saved.selectedId || !saved.candidates.some((candidate) => candidate.videoId === saved.selectedId && youtubeId(candidate.url) === saved.selectedId)))) {
      saved.confirmed = null;
      saved.stage = "choosing";
      saved.error = "Choose a match and check its audio again.";
    }
    this.publish(saved);
  }

  dispose(): void { this.disposed = true; ++this.epoch; }
}
