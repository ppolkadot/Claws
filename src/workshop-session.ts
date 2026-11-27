/**
 * One Wallpaper Engine item on the Download screen: the export while it runs, and how it
 * ended. The project always comes from the user's own Steam library. Kept in the store, so leaving Download and coming back
 * shows a running export as it is. No DOM here; views/workshop.ts draws it.
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

export type WorkshopStage = "starting" | "converting" | "saving";

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

export interface LibraryStatus {
  /** Steam is installed for this Windows user. */
  steam: boolean;
  /** The item's project is in a Steam library. */
  found: boolean;
  /** The local manifest names a supported video file. */
  video: boolean;
}

export type WorkshopState =
  | { phase: "ready" }
  | { phase: "running"; stage: WorkshopStage; percent: number | null }
  | { phase: "saved"; filename: string; path: string }
  | { phase: "failed"; message: string; detail: string | null }
  /** The chosen video format does not fit this project; saving the ZIP still works. */
  | { phase: "unsupported"; message: string }
  | { phase: "cancelled" };

export interface WorkshopDeps {
  download(url: string, format: ExportFormat): Promise<WallpaperOutcome>;
  cancel(): Promise<void>;
  findInLibrary(url: string): Promise<LibraryStatus>;
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
  return null;
}

/** How often, and for how long, a subscription in Steam is waited for. */
export const LIBRARY_POLL_MS = 3000;
export const LIBRARY_WAIT_MS = 10 * 60 * 1000;

export class WorkshopSession {
  library: LibraryStatus | null = null;
  state: WorkshopState = { phase: "ready" };
  private chosenFormat: ExportFormat | null = null;
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

  /** Reads the Steam library once. */
  async checkLibrary(): Promise<void> {
    try {
      this.library = await this.deps.findInLibrary(this.url);
    } catch {
      this.library = { steam: false, found: false, video: false };
    }
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
      if (this.library?.found || Date.now() > this.watchUntil) this.stopWatching();
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
      outcome = await this.deps.download(this.url, this.format);
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
