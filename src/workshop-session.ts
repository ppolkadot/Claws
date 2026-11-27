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

/**
 * ZIP saves the whole project. For a video wallpaper MP4 and GIF are its own file or a conversion
 * of it; for a scene or web wallpaper MP4 is a recording and PNG is one picture.
 */
export type ExportFormat = "zip" | "mp4" | "gif" | "png";

export type ProjectKind = "video" | "scene" | "web" | "application" | "preset" | "unknown";

/** One size a recording can be made at. */
export interface SizeOption {
  width: number;
  height: number;
  label: string;
  available: boolean;
  /** Why it cannot be chosen. */
  reason: string | null;
  /** Larger than the display reports a window can be; Claws still tries and says if it cannot. */
  beyondDisplay: boolean;
  /** The frame rates this size can be recorded at. */
  fps: number[];
}

/** What Claws knows about recording a scene or web wallpaper on this PC. */
export interface RecordInfo {
  kind: "scene" | "web";
  available: boolean;
  reason: string | null;
  /** The scene changes by itself; null for a web page. */
  moves: boolean | null;
  hasAudio: boolean;
  sizes: SizeOption[];
  defaultHeight: number | null;
  preRollSeconds: number;
  encoder: string | null;
  /** Wallpaper Engine is not running now and will be started. */
  engineWillStart: boolean;
  displayLimit: [number, number] | null;
}

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
  | { status: "saved"; filename: string; path: string; elapsedMs: number; stages: StageTime[]; note?: string | null }
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
  /** What kind of project it is. */
  kind: ProjectKind;
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
  /** GIF: seconds from the start (30 by default, 60 at most). Recording: how long (30, 600 at most). */
  lengthSeconds?: number;
  /** Recording: the height in pixels. */
  height?: number;
  /** Recording: 30 or 60. */
  fps?: number;
}

/** The lengths offered for a recording, besides a custom one up to ten minutes. */
export const RECORD_LENGTHS = [10, 30, 60] as const;
export const RECORD_MAX_SECONDS = 600;

export const GIF_SECONDS = [30, 60] as const;

export type WorkshopState =
  | { phase: "ready" }
  | { phase: "running"; progress: ExportProgress; /** What the bar shows: null is the sheen. */ fraction: number | null }
  | { phase: "saved"; filename: string; path: string; elapsedMs: number; stages: StageTime[]; note?: string | null }
  | { phase: "failed"; message: string; detail: string | null }
  /** The chosen video format does not fit this project; saving the ZIP still works. */
  | { phase: "unsupported"; message: string }
  | { phase: "cancelled" };

export interface WorkshopDeps {
  download(url: string, format: ExportFormat, options: ExportOptions): Promise<WallpaperOutcome>;
  cancel(): Promise<void>;
  findInLibrary(url: string, timeUpdated: number | null): Promise<LibraryStatus>;
  videoInfo(url: string): Promise<VideoInfo | null>;
  recordInfo(url: string): Promise<RecordInfo | null>;
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
  const percent = counted === null ? null : `${Math.min(99, Math.round(counted * 100))}%`;
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
  if (library.video) return "Video found in your Steam library.";
  if (library.kind === "scene") return "Scene found in your Steam library.";
  if (library.kind === "web") return "Web wallpaper found in your Steam library.";
  return "Project found. Claws can save a ZIP.";
}

export interface FormatChoice {
  value: ExportFormat;
  label: string;
  title: string;
}

/** What can be done with the project besides zipping it. */
export interface FormatOffer {
  /** The project has a video Claws can save. */
  video: boolean;
  videoInfo: VideoInfo | null;
  /** A scene or web wallpaper that Wallpaper Engine can play for a recording. */
  record: RecordInfo | null;
}

/**
 * The formats offered. A video wallpaper: ZIP, MP4 (its own file when it is H.264, converted
 * otherwise) and GIF. A scene or web wallpaper that can be recorded: MP4 (recorded), a picture
 * (PNG) and ZIP. Anything else: ZIP only.
 */
export function formatChoices(offer: FormatOffer): FormatChoice[] {
  const zip: FormatChoice = { value: "zip", label: "ZIP", title: "The whole project, as Steam downloaded it" };
  if (offer.record?.available) {
    return [
      { value: "mp4", label: "MP4 (recorded)", title: "Wallpaper Engine plays it in the background and Claws records the video" },
      { value: "png", label: "Picture (PNG)", title: "One picture of the wallpaper" },
      zip,
    ];
  }
  if (!offer.video) return [zip];
  return [
    zip,
    offer.videoInfo?.copyable
      ? { value: "mp4", label: "MP4 (original)", title: "The wallpaper's own video file, saved unchanged" }
      : { value: "mp4", label: "MP4 (converted)", title: "H.264 video with its audio, converted so it plays anywhere" },
    { value: "gif", label: "GIF", title: "A silent looping GIF of the first seconds, at most 720p" },
  ];
}

/** The size recorded at: the one asked for if it can be had, else the default. */
export function recordSize(record: RecordInfo | null, height: number | null): SizeOption | null {
  if (!record) return null;
  const asked = height === null ? undefined : record.sizes.find((size) => size.height === height && size.available);
  return asked ?? record.sizes.find((size) => size.height === record.defaultHeight) ?? null;
}

/** The frame rate recorded at: the one asked for if this size allows it, else 30. */
export function recordFps(size: SizeOption | null, fps: number): number {
  return size?.fps.includes(fps) ? fps : 30;
}

/** The short notes under a recording's options: what it is, and what it will not do. */
export function recordNotes(record: RecordInfo, format: ExportFormat, size: SizeOption | null): string[] {
  const notes: string[] = [];
  if (record.moves === false) {
    notes.push(format === "png" ? "This wallpaper doesn't move." : "This wallpaper doesn't move, so a recording would be a still video. A picture is better.");
  }
  if (record.hasAudio) notes.push("This scene has sound. The recording is silent.");
  if (size?.beyondDisplay && record.displayLimit) {
    notes.push(
      `${size.label} may need a display set to ${size.width}×${size.height} or larger, such as a 4K monitor or NVIDIA DSR / AMD VSR. Claws tells you if the window can't reach it.`,
    );
  }
  if (record.engineWillStart) notes.push("Wallpaper Engine will start.");
  return notes;
}

/** How often, and for how long, a subscription in Steam is waited for. */
export const LIBRARY_POLL_MS = 3000;
export const LIBRARY_WAIT_MS = 10 * 60 * 1000;

export class WorkshopSession {
  library: LibraryStatus | null = null;
  /** What the video is, read once the library has it. */
  video: VideoInfo | null = null;
  /** What recording a scene or web wallpaper takes, read once the library has it. */
  record: RecordInfo | null = null;
  /** The recording length in seconds, and whether the user typed their own. */
  recordLength: number = RECORD_LENGTHS[1];
  lengthCustom = false;
  /** The height asked for (null: the default) and the frame rate. */
  recordHeight: number | null = null;
  recordFpsChoice = 30;
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

  /** The formats on offer for this project, in the order they are shown. */
  get choices(): FormatChoice[] {
    return formatChoices({ video: this.videoAvailable, videoInfo: this.video, record: this.record });
  }

  /** More to choose than the ZIP: a video's formats, or a recording. */
  get offersMore(): boolean {
    return this.choices.length > 1;
  }

  get format(): ExportFormat {
    const values = this.choices.map((choice) => choice.value);
    if (values.length <= 1) return "zip";
    if (this.chosenFormat && values.includes(this.chosenFormat)) return this.chosenFormat;
    // A picture that does not move is best as a picture.
    return this.record?.available && this.record.moves === false ? "png" : "mp4";
  }

  /** Recording a scene or web wallpaper (MP4 or a picture), rather than saving a video. */
  get recording(): boolean {
    return Boolean(this.record?.available) && (this.format === "mp4" || this.format === "png");
  }

  get size(): SizeOption | null {
    return recordSize(this.record, this.recordHeight);
  }

  get fps(): number {
    return recordFps(this.size, this.recordFpsChoice);
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
      this.library = { steam: false, wallpaperEngine: false, found: false, video: false, kind: "unknown", state: "notInstalled" };
    }
    if (this.library.found && this.library.video && !this.video) {
      try {
        this.video = await this.deps.videoInfo(this.url);
      } catch {
        this.video = null;
      }
    }
    if (this.library.found && (this.library.kind === "scene" || this.library.kind === "web") && !this.record) {
      try {
        this.record = await this.deps.recordInfo(this.url);
      } catch {
        this.record = null;
      }
    }
    this.changed();
    if (this.estimate === null) void this.refreshEstimate();
  }

  /** The options that go with the chosen format. */
  get options(): ExportOptions {
    if (this.format === "gif") return { lengthSeconds: this.gifSeconds };
    if (this.recording && this.format === "png") return { height: this.size?.height };
    if (this.recording) return { lengthSeconds: this.recordLength, height: this.size?.height, fps: this.fps };
    return {};
  }

  private optionChanged() {
    this.estimate = null;
    if (this.state.phase !== "ready") this.state = { phase: "ready" };
    this.changed();
    void this.refreshEstimate();
  }

  /** A recording's length, a whole number of seconds from one to ten minutes. */
  setRecordLength(seconds: number, custom = false) {
    if (this.running || !Number.isFinite(seconds)) return;
    const next = Math.min(RECORD_MAX_SECONDS, Math.max(1, Math.round(seconds)));
    if (next === this.recordLength && custom === this.lengthCustom) return;
    this.recordLength = next;
    this.lengthCustom = custom;
    this.optionChanged();
  }

  /** Switches between the lengths on offer and typing one's own. */
  setLengthCustom(custom: boolean) {
    if (this.running || this.lengthCustom === custom) return;
    this.lengthCustom = custom;
    this.optionChanged();
  }

  setRecordHeight(height: number) {
    if (this.running || this.recordHeight === height || !this.record?.sizes.some((size) => size.height === height && size.available)) return;
    this.recordHeight = height;
    this.optionChanged();
  }

  setRecordFps(fps: number) {
    if (this.running || this.recordFpsChoice === fps || !this.size?.fps.includes(fps)) return;
    this.recordFpsChoice = fps;
    this.optionChanged();
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
    if (this.running || !this.choices.some((choice) => choice.value === format)) return;
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
        ? { phase: "saved", filename: outcome.filename, path: outcome.path, elapsedMs: outcome.elapsedMs, stages: outcome.stages, note: outcome.note ?? null }
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
