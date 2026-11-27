import { clock, folderName, formatBytes, formatBytesOf, formatDuration, formatEta } from "./format.js";

/**
 * One Wallpaper Engine item on the Download screen: the export while it runs, and how it
 * ended. The project always comes from the user's own Steam library. Kept in the store, so
 * leaving Download and coming back shows a running export as it is. No DOM here;
 * views/workshop.ts draws it.
 */

export interface WorkshopItem {
  id: string;
  title: string;
  previewUrl: string | null;
  fileSize: number | null;
  timeUpdated: number | null;
  /** "Scene", "Video", "Web", "Application" or "Preset". */
  wallpaperType: string | null;
  /** Other Workshop items this one needs. */
  requiredItems: string[];
}

export type WorkshopLookup =
  | { kind: "wallpaper"; item: WorkshopItem }
  | { kind: "collection"; title: string; items: number }
  | { kind: "otherApp"; appId: number }
  | { kind: "notFound" };

/** ZIP saves the whole project; MP4 and GIF are converted from a video wallpaper. */
export type ExportFormat = "zip" | "mp4" | "gif";

export type WorkshopStage = "checking" | "opening" | "loading" | "recording" | "finishing" | "copying" | "converting" | "zipping";

/** The `workshop-progress` event. A number is there only when the backend can stand behind it. */
export interface ExportProgress {
  url: string;
  stage: WorkshopStage;
  done: number | null;
  total: number | null;
  unit: "seconds" | "bytes" | null;
  /** Milliseconds left; null when there is no estimate. */
  etaMs: number | null;
  /** The time left rests on a measured rate, so it is shown as "about". */
  approximate: boolean;
  elapsedMs: number;
  /** Recording: the size of the video so far. */
  bytes?: number | null;
}

export type WorkshopProgress = ExportProgress;

export interface StageTime {
  name: string;
  ms: number;
}

export type WallpaperOutcome =
  | { status: "saved"; filename: string; path: string; elapsedMs: number; stages: StageTime[] }
  | { status: "failed"; message: string; detail: string | null }
  | { status: "unsupported"; message: string }
  | { status: "cancelled" };

/** What Steam says about the item: installed, still downloading (bytes on disk), or updated since. */
export type LibraryItemState =
  | { state: "notInstalled" }
  | { state: "installed" }
  | { state: "downloading"; bytes: number }
  | { state: "updatePending" };

export type LibraryStatus = {
  /** Steam is installed for this Windows user. */
  steam: boolean;
  /** Wallpaper Engine is installed in one of Steam's libraries. */
  wallpaperEngine: boolean;
  /** The item's project is in a Steam library. */
  found: boolean;
  /** The local manifest names a supported video file. */
  video: boolean;
} & LibraryItemState;

/** What the video is: whether the original can be saved as it is, and its size and length. */
export interface VideoInfo {
  /** 8-bit H.264 in an MP4: "MP4 (original)" copies it. Otherwise MP4 is converted. */
  copyable: boolean;
  codec: string;
  width: number;
  height: number;
  fps: number;
  durationSeconds: number | null;
  bytes: number;
  hasAudio: boolean;
}

/** What a video export may be asked for beyond its format. */
export interface ExportOptions {
  /** GIF: seconds from the start (30 by default, 60 at most). */
  lengthSeconds?: number;
}

export const GIF_SECONDS = [30, 60] as const;

export type WorkshopState =
  | { phase: "ready" }
  | { phase: "running"; progress: ExportProgress; /** What the bar shows: null is the sheen. */ fraction: number | null }
  | { phase: "saved"; filename: string; path: string; elapsedMs: number; stages: StageTime[] }
  | { phase: "failed"; message: string; detail: string | null }
  /** The chosen video format does not fit this project; saving the ZIP still works. */
  | { phase: "unsupported"; message: string }
  | { phase: "cancelled" };

export interface WorkshopDeps {
  download(url: string, format: ExportFormat, options: ExportOptions): Promise<WallpaperOutcome>;
  cancel(): Promise<void>;
  findInLibrary(url: string, timeUpdated: number | null): Promise<LibraryStatus>;
  videoInfo(url: string): Promise<VideoInfo | null>;
  estimate(url: string, format: ExportFormat, options: ExportOptions): Promise<ExportEstimate>;
  describeError(error: unknown): string;
}

/** What a Workshop lookup that is not a wallpaper says. */
export function lookupProblem(found: WorkshopLookup): string | null {
  switch (found.kind) {
    case "wallpaper":
      return null;
    case "collection":
      return "This link is a Workshop collection. Open each wallpaper in it on its own.";
    case "otherApp":
      return "Claws downloads Workshop items for Wallpaper Engine only.";
    case "notFound":
      return "Steam couldn't find this Workshop item. It may be private or removed.";
  }
}

/** What the export is expected to take, shown under the format choice before it starts. */
export interface ExportEstimate {
  seconds: number | null;
  approximate: boolean;
  bytes: number | null;
}

/** While running, the bar never shows more than this: 100% is for the saved file. */
export const BAR_CAP = 0.99;

/** done over total, within 0 to 1; null when the stage counts nothing or is only finishing. */
export function stageFraction(p: ExportProgress): number | null {
  if (p.stage === "finishing") return null;
  if (p.done === null || p.total === null || !(p.total > 0) || !Number.isFinite(p.done)) return null;
  return Math.min(1, Math.max(0, p.done / p.total));
}

/**
 * What the bar shows next. A counted stage shows its fraction, never below what was already
 * shown and never past `BAR_CAP`. Finishing holds the last value rather than going back to
 * the sheen; a stage with nothing to count shows the sheen.
 */
export function barFraction(previous: number | null, p: ExportProgress): number | null {
  if (p.stage === "finishing") return previous;
  const fraction = stageFraction(p);
  if (fraction === null) return null;
  return Math.min(BAR_CAP, Math.max(previous ?? 0, fraction));
}

/** "about 3:10 left", "18s left", or "" when there is no estimate. */
export function etaText(etaMs: number | null, approximate: boolean): string {
  if (etaMs === null || !Number.isFinite(etaMs) || etaMs < 0) return "";
  return `${approximate ? "about " : ""}${formatEta(etaMs / 1000)}`;
}

const SEPARATOR = "  ·  ";

/** The status line while an export runs: the stage, the count, and the time left. */
export function stageLine(p: ExportProgress): string {
  const left = etaText(p.etaMs, p.approximate);
  const counted = stageFraction(p);
  const percent = counted === null ? null : `${Math.round(counted * 100)}%`;
  const join = (...parts: (string | null | undefined)[]) => parts.filter(Boolean).join(SEPARATOR);
  switch (p.stage) {
    case "checking":
      return "Checking the wallpaper…";
    case "opening":
      return "Opening in Wallpaper Engine…";
    case "loading":
      return "Loading the scene…";
    case "finishing":
      return "Finishing the file…";
    case "recording": {
      const counting = p.done !== null && p.total !== null ? `Recording ${clock(p.done)} of ${clock(p.total)}` : "Recording…";
      return join(counting, p.bytes ? formatBytes(p.bytes) : null, left);
    }
    case "copying":
      return join(p.done !== null && p.total !== null ? `Copying ${formatBytesOf(p.done, p.total)}` : "Copying…", left);
    case "converting":
      return join(percent ? `Converting ${percent}` : "Converting…", left);
    case "zipping":
      return join(percent ? `Saving the ZIP ${percent}` : "Saving the ZIP…", left);
  }
}

/** "took 1:03": the whole export, never less than a second. */
export function tookText(elapsedMs: number): string {
  return `took ${formatDuration(Math.max(1, Math.round(elapsedMs / 1000)))}`;
}

/** The folder part of a saved file's path. */
function folderOf(path: string): string {
  return path.replace(/[\\/][^\\/]*$/, "");
}

/** "Saved to Videos  ·  took 1:03". */
export function savedLine(path: string, elapsedMs: number): string {
  return `Saved to ${folderName(folderOf(path))}${SEPARATOR}${tookText(elapsedMs)}`;
}

/** The estimate under the format choice: the time, and the size when it is known. */
export function estimateLine(estimate: ExportEstimate | null): string {
  if (!estimate) return "";
  if (estimate.seconds === null) return "Estimate after a few seconds";
  const time = `About ${clock(Math.max(1, estimate.seconds))}`;
  return [time, estimate.bytes ? `about ${formatBytes(estimate.bytes)}` : null].filter(Boolean).join(SEPARATOR);
}

/** Whether an export over this long is the kind that gets a confirmation first (GIF). */
export const CONFIRM_AFTER_SECONDS = 10 * 60;

/** The sentence for the confirmation sheet when an estimate runs long. */
export function longRunMessage(estimate: ExportEstimate): string | null {
  if (estimate.seconds === null || estimate.seconds <= CONFIRM_AFTER_SECONDS) return null;
  return `This is expected to take about ${clock(estimate.seconds)}. You can keep using Claws while it runs, and cancel at any time.`;
}

/** Video projects can be saved as MP4 or GIF; every other project only as its ZIP. */
export function offersVideo(item: WorkshopItem): boolean {
  return item.wallpaperType?.toLowerCase() === "video";
}

/** Why the export can't start yet, or null when it can. */
export function missingForStart(library: LibraryStatus | null): string | null {
  if (library && !library.steam) return "Steam isn't installed for this Windows user.";
  if (!library?.found) return "Subscribe in Steam first.";
  if (library.state === "downloading") return "Steam is still downloading it.";
  return null;
}

/** How far Steam's download is, as a fraction below 1 (the size Valve lists is not exact). */
export function downloadFraction(library: LibraryStatus | null, fileSize: number | null): number | null {
  if (library?.state !== "downloading" || !fileSize || fileSize <= 0) return null;
  return Math.min(0.99, Math.max(0, library.bytes / fileSize));
}

/** The one line that says where the item stands in Steam. */
export function libraryLine(library: LibraryStatus | null, fileSize: number | null, watching: boolean): string {
  if (!library) return "Checking…";
  if (!library.steam) return "Claws needs Steam with Wallpaper Engine installed on this PC, and nothing else.";
  if (!library.found && library.state !== "downloading" && !library.wallpaperEngine) {
    return "Wallpaper Engine isn't installed in your Steam library. Claws needs Steam with Wallpaper Engine, and nothing else.";
  }
  const fraction = downloadFraction(library, fileSize);
  if (library.state === "downloading") return fraction === null ? "Steam is downloading it…" : `Steam is downloading it… ${Math.round(fraction * 100)}%`;
  if (!library.found) return watching ? "Waiting for Steam to download it…" : "Not in your library yet.";
  if (library.state === "updatePending") return "Steam has an update for this wallpaper. Claws will save the version you have.";
  return library.video ? "Video found in your Steam library." : "Project found. Claws can save a ZIP. MP4 and GIF need a standalone video.";
}

export interface FormatChoice {
  value: ExportFormat;
  label: string;
  title: string;
}

/** The formats a video wallpaper offers. MP4 says whether it saves the original or converts. */
export function formatChoices(videoAvailable: boolean, video: VideoInfo | null): FormatChoice[] {
  const zip: FormatChoice = { value: "zip", label: "ZIP", title: "The whole project, as Steam downloaded it" };
  if (!videoAvailable) return [zip];
  return [
    zip,
    video?.copyable
      ? { value: "mp4", label: "MP4 (original)", title: "The wallpaper's own video file, saved unchanged" }
      : { value: "mp4", label: "MP4 (converted)", title: "H.264 video with its audio, converted so it plays anywhere" },
    { value: "gif", label: "GIF", title: "A silent looping GIF of the first seconds, at most 720p" },
  ];
}

/** How often, and for how long, a subscription in Steam is waited for. */
export const LIBRARY_POLL_MS = 3000;
export const LIBRARY_WAIT_MS = 10 * 60 * 1000;

export class WorkshopSession {
  library: LibraryStatus | null = null;
  /** What the video is, read once the library has it. */
  video: VideoInfo | null = null;
  state: WorkshopState = { phase: "ready" };
  /** What the bar last showed, kept after a failure or a cancel (the bar turns red there). */
  lastFraction: number | null = null;
  /** What the chosen format is expected to take; null until asked. */
  estimate: ExportEstimate | null = null;
  private estimateTicket = 0;
  private chosenFormat: ExportFormat | null = null;
  /** GIF length in seconds. */
  gifSeconds: number = GIF_SECONDS[0];
  /** Polling the Steam library after the user went to subscribe. */
  watching = false;
  private watchTimer: ReturnType<typeof setTimeout> | null = null;
  private watchUntil = 0;
  private listeners = new Set<() => void>();

  constructor(
    readonly url: string,
    readonly item: WorkshopItem,
    private readonly deps: WorkshopDeps,
  ) {}

  get running(): boolean {
    return this.state.phase === "running";
  }

  get videoAvailable(): boolean {
    return this.library?.video === true;
  }

  get format(): ExportFormat {
    if (!this.videoAvailable) return "zip";
    return this.chosenFormat ?? "mp4";
  }

  on(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private changed() {
    for (const listener of [...this.listeners]) listener();
  }

  /** Reads the Steam library once, and what the video is the first time it is there. */
  async checkLibrary(): Promise<void> {
    try {
      this.library = await this.deps.findInLibrary(this.url, this.item.timeUpdated);
    } catch {
      this.library = { steam: false, wallpaperEngine: false, found: false, video: false, state: "notInstalled" };
    }
    if (this.library.found && this.library.video && !this.video) {
      try {
        this.video = await this.deps.videoInfo(this.url);
      } catch {
        this.video = null;
      }
    }
    this.changed();
    if (this.estimate === null) void this.refreshEstimate();
  }

  /** The options that go with the chosen format. */
  get options(): ExportOptions {
    return this.format === "gif" ? { lengthSeconds: this.gifSeconds } : {};
  }

  /**
   * Asks the backend what the chosen format is expected to take. An answer for a choice the
   * user has since changed is dropped.
   */
  async refreshEstimate(): Promise<void> {
    if (!this.library?.found || this.running) return;
    const ticket = ++this.estimateTicket;
    let estimate: ExportEstimate | null;
    try {
      estimate = await this.deps.estimate(this.url, this.format, this.options);
    } catch {
      estimate = null;
    }
    if (ticket !== this.estimateTicket) return;
    this.estimate = estimate;
    this.changed();
  }

  setGifSeconds(seconds: number) {
    if (this.running || this.gifSeconds === seconds || !GIF_SECONDS.some((option) => option === seconds)) return;
    this.gifSeconds = seconds;
    this.estimate = null;
    if (this.state.phase !== "ready") this.state = { phase: "ready" };
    this.changed();
    void this.refreshEstimate();
  }

  /** Switches the export format, clearing an old outcome so the panel starts fresh. */
  setFormat(format: ExportFormat) {
    if (this.running || (format !== "zip" && !this.videoAvailable)) return;
    if (this.chosenFormat === format) return;
    this.chosenFormat = format;
    this.estimate = null;
    if (this.state.phase !== "ready") this.state = { phase: "ready" };
    this.changed();
    void this.refreshEstimate();
  }

  /** After "Subscribe in Steam": checks the library until the project appears. */
  watchLibrary(now = Date.now()) {
    this.stopWatching();
    this.watching = true;
    this.watchUntil = now + LIBRARY_WAIT_MS;
    this.changed();
    this.scheduleWatch();
  }

  stopWatching() {
    if (this.watchTimer) clearTimeout(this.watchTimer);
    this.watchTimer = null;
    if (this.watching) {
      this.watching = false;
      this.changed();
    }
  }

  private scheduleWatch() {
    this.watchTimer = setTimeout(async () => {
      this.watchTimer = null;
      if (!this.watching) return;
      await this.checkLibrary();
      if (!this.watching) return;
      // Steam may still be writing the item after its folder appears: wait for it to finish.
      const done = this.library?.found === true && this.library.state !== "downloading";
      if (done || Date.now() > this.watchUntil) this.stopWatching();
      else this.scheduleWatch();
    }, LIBRARY_POLL_MS);
  }

  async start(): Promise<void> {
    if (this.running) return;
    this.stopWatching();
    this.estimateTicket += 1;
    this.lastFraction = null;
    this.state = {
      phase: "running",
      fraction: null,
      progress: { url: this.url, stage: "checking", done: null, total: null, unit: null, etaMs: null, approximate: false, elapsedMs: 0 },
    };
    this.changed();
    let outcome: WallpaperOutcome;
    try {
      outcome = await this.deps.download(this.url, this.format, this.options);
    } catch (error) {
      outcome = { status: "failed", message: this.deps.describeError(error), detail: null };
    }
    this.state =
      outcome.status === "saved"
        ? { phase: "saved", filename: outcome.filename, path: outcome.path, elapsedMs: outcome.elapsedMs, stages: outcome.stages }
        : outcome.status === "failed"
          ? { phase: "failed", message: outcome.message, detail: outcome.detail }
          : outcome.status === "unsupported"
            ? { phase: "unsupported", message: outcome.message }
            : { phase: "cancelled" };
    this.changed();
  }

  cancel() {
    if (this.running) void this.deps.cancel();
  }

  /** A `workshop-progress` event from the backend. */
  progress(event: WorkshopProgress) {
    if (event.url !== this.url || this.state.phase !== "running") return;
    const fraction = barFraction(this.state.fraction, event);
    this.lastFraction = fraction;
    this.state = { phase: "running", progress: event, fraction };
    this.changed();
  }

  /** What the taskbar button shows for this export: nothing, the sheen (null) or a fraction. */
  get activity(): { running: boolean; fraction: number | null } {
    return this.state.phase === "running" ? { running: true, fraction: this.state.fraction } : { running: false, fraction: null };
  }

  dispose() {
    this.stopWatching();
    this.listeners.clear();
  }
}
