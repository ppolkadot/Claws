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

export type WorkshopStage = "starting" | "copying" | "converting" | "saving";

export interface WorkshopProgress {
  url: string;
  stage: WorkshopStage;
  percent: number | null;
}

export type WallpaperOutcome =
  | { status: "saved"; filename: string; path: string }
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
  | { phase: "running"; stage: WorkshopStage; percent: number | null }
  | { phase: "saved"; filename: string; path: string }
  | { phase: "failed"; message: string; detail: string | null }
  /** The chosen video format does not fit this project; saving the ZIP still works. */
  | { phase: "unsupported"; message: string }
  | { phase: "cancelled" };

export interface WorkshopDeps {
  download(url: string, format: ExportFormat, options: ExportOptions): Promise<WallpaperOutcome>;
  cancel(): Promise<void>;
  findInLibrary(url: string, timeUpdated: number | null): Promise<LibraryStatus>;
  videoInfo(url: string): Promise<VideoInfo | null>;
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

/** The status line while an export runs. */
export function stageText(stage: WorkshopStage, percent: number | null, format: ExportFormat = "zip"): string {
  const pct = percent === null ? "" : ` ${Math.round(percent)}%`;
  switch (stage) {
    case "starting":
      return "Starting…";
    case "copying":
      return `Copying the video…${pct}`;
    case "converting":
      return "Converting the video with FFmpeg…";
    case "saving":
      return format === "zip" ? `Saving the ZIP…${pct}` : `Saving the ${format.toUpperCase()}…${pct}`;
  }
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
  }

  setGifSeconds(seconds: number) {
    if (this.running || this.gifSeconds === seconds || !GIF_SECONDS.some((option) => option === seconds)) return;
    this.gifSeconds = seconds;
    if (this.state.phase !== "ready") this.state = { phase: "ready" };
    this.changed();
  }

  /** Switches the export format, clearing an old outcome so the panel starts fresh. */
  setFormat(format: ExportFormat) {
    if (this.running || (format !== "zip" && !this.videoAvailable)) return;
    if (this.chosenFormat === format) return;
    this.chosenFormat = format;
    if (this.state.phase !== "ready") this.state = { phase: "ready" };
    this.changed();
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
    this.state = { phase: "running", stage: "starting", percent: null };
    this.changed();
    let outcome: WallpaperOutcome;
    try {
      outcome = await this.deps.download(this.url, this.format, this.format === "gif" ? { lengthSeconds: this.gifSeconds } : {});
    } catch (error) {
      outcome = { status: "failed", message: this.deps.describeError(error), detail: null };
    }
    this.state =
      outcome.status === "saved"
        ? { phase: "saved", filename: outcome.filename, path: outcome.path }
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
    this.state = { phase: "running", stage: event.stage, percent: event.percent };
    this.changed();
  }

  dispose() {
    this.stopWatching();
    this.listeners.clear();
  }
}
