import {
  api,
  errorMessage,
  type DownloadMode,
  type DownloadOptions,
  type HistoryItem,
  type Job,
  type JobEvent,
  type JobRemoved,
  type JobsAdded,
  type ProbeResult,
  type Settings,
  type ToolProgress,
  type ToolsStatus,
  type TranscriptFormat,
  type VideoPreset,
} from "./api";
import type { RunSnapshot } from "./download-estimates";
import { LinkBatch, type ParsedInput } from "./download-input";
import { PreviewMemory } from "./preview-session";
import { JobTable } from "./queue-events";

export type Route = "download" | "queue" | "library" | "settings";

/** The choices shared by every link in a batch. Batches have no subtitle choices. */
export interface BatchChoices {
  mode: "video" | "audio";
  maxHeight: number | null;
  videoPreset: VideoPreset;
  audioFormat: string;
}

/** One item whose download size is estimated, with the choices it was asked for. */
export interface EstimateItem {
  url: string;
  playlistIndex: number | null;
  options: DownloadOptions;
}

/** The choices made over one link's details, as plain data (the sets the view uses are arrays). */
export interface PreviewChoices {
  mode: DownloadMode;
  height: number | null;
  videoPreset: VideoPreset;
  audioFormat: string;
  subs: string[];
  embedSubs: boolean;
  fastSave: boolean;
  caption: string | null;
  transcriptFormat: TranscriptFormat;
}

/** Several links pasted at once, being looked up or waiting to be added to the queue. */
export interface BatchSession {
  /** The submitted text's parse, kept for its skipped lines and repeats. */
  parsed: ParsedInput;
  choices: BatchChoices;
  /**
   * The batch's finished estimate run, saved while another view is open and taken back once
   * when the Download view returns. Never a run still going.
   */
  estimate: RunSnapshot<EstimateItem> | null;
}

type Topic =
  | "route"
  /** The list's structure changed: jobs were added or removed. */
  | "jobs"
  /** One known job changed (detail: the job). */
  | "job"
  /** A job's status changed, or jobs were added or removed: counts may differ. */
  | "jobStatus"
  | "history"
  | "settings"
  /** A save was rejected and some fields went back to their saved values. */
  | "settingsReverted"
  /** A link was pasted somewhere in the window; see `takePastedLink`. */
  | "paste"
  /** Quick Open without a link: put the cursor in the link field. */
  | "focusLink"
  | "tools"
  | "toolProgress"
  /** A batch of links started or was cleared; see `store.batch`. */
  | "batch"
  /** One row of the batch changed (detail: the row). */
  | "batchRow";

const sameValue = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
type Listener = (detail?: unknown) => void;

const defaultSettings: Settings = {
  schemaVersion: 2,
  outputDir: "",
  filenameTemplate: "%(title).150B [%(id)s].%(ext)s",
  cookieSource: { kind: "none", value: "" },
  theme: "system",
  glassIntensity: 70,
  backdrop: "glass",
  maxConcurrent: 3,
  embedMetadata: true,
  quickOpenShortcut: "ctrl+alt+d",
  keepRunning: true,
  openAtLogin: false,
  audioSelection: "preferMatching",
};

class Store {
  route: Route = "download";
  settings: Settings = defaultSettings;
  /** The download list, kept in creation order from revisioned events. */
  readonly jobs = new JobTable<Job>();
  history: HistoryItem[] = [];
  tools: ToolsStatus = { ytdlp: null, ffmpeg: null, latestYtdlp: null, jsRuntime: null };
  toolProgress: Partial<Record<"ytdlp" | "ffmpeg", ToolProgress>> = {};
  toolBusy: Partial<Record<"ytdlp" | "ffmpeg", boolean>> = {};
  toolError: Partial<Record<"ytdlp" | "ffmpeg", string>> = {};

  /** False until the first tool check returns; until then "not installed" is unknown. */
  toolsKnown = false;
  private toolsReadyResolve!: () => void;
  /** Resolves once the first tool check has returned. */
  toolsReady: Promise<void> = new Promise((resolve) => (this.toolsReadyResolve = resolve));

  private listeners = new Map<Topic, Set<Listener>>();
  private saveTimer: number | undefined;

  // Every edit bumps `revision`; saves run one at a time on `saveChain`, and a response
  // is only adopted if nothing was edited since, so an older save never wins.
  /** The last settings the backend accepted. */
  private confirmed: Settings = defaultSettings;
  private revision = 0;
  private sentRevision = 0;
  private saveChain: Promise<void> = Promise.resolve();
  private onSaveError: (message: string) => void = () => undefined;
  private lastWarning: string | null = null;

  private pastedLink: string | null = null;

  /**
   * The single link being worked on in the Download view: its details, choices and estimates,
   * kept while Queue or Settings is open. Replaced by another link or a batch, and ended when the
   * link is added to the queue (preview-session.ts).
   */
  readonly previews = new PreviewMemory<ProbeResult, PreviewChoices, EstimateItem>();

  /**
   * The current batch. It lives here, not in the Download view, so its lookups carry on
   * and its rows stay while another view is open. One scheduler for every batch, so
   * lookups still running from a replaced batch count towards its limit of three.
   */
  batch: BatchSession | null = null;
  readonly links = new LinkBatch<ProbeResult>((url) => api.probe(url), {
    onRow: (row) => this.emit("batchRow", row),
    describe: errorMessage,
  });

  on(topic: Topic, listener: Listener): () => void {
    let set = this.listeners.get(topic);
    if (!set) this.listeners.set(topic, (set = new Set()));
    set.add(listener);
    return () => set.delete(listener);
  }

  emit(topic: Topic, detail?: unknown) {
    this.listeners.get(topic)?.forEach((listener) => listener(detail));
  }

  /** In creation order. The same array until jobs are added or removed. */
  jobList(): Job[] {
    return this.jobs.list();
  }

  /** Finished or cancelled jobs, which Clear Finished would remove. */
  finishedCount(): number {
    let count = 0;
    for (const job of this.jobs.values()) {
      if (job.status === "completed" || job.status === "cancelled") count += 1;
    }
    return count;
  }

  activeCount(): number {
    let count = 0;
    for (const job of this.jobs.values()) {
      if (job.status === "queued" || job.status === "downloading" || job.status === "processing") {
        count += 1;
      }
    }
    return count;
  }

  go(route: Route) {
    if (route === this.route) return;
    this.route = route;
    this.emit("route");
  }

  async load() {
    const data = await api.bootstrap();
    this.settings = data.settings;
    this.confirmed = data.settings;
    this.jobs.load(data.jobs, (job) => job.revision ?? 0);
    this.history = data.history;
    return data.notices;
  }

  /** Checks the tools in the background; the first frame never waits for this. */
  async loadTools() {
    try {
      this.tools = await api.toolsStatus();
    } finally {
      this.toolsKnown = true;
      this.toolsReadyResolve();
      this.emit("tools");
    }
  }

  /** One job changed (a `job-update` event). Older or repeated revisions are ignored. */
  updateJob(event: JobEvent<Job>) {
    const change = this.jobs.apply(event);
    if (change.kind === "ignored") return;
    if (change.kind === "added") {
      this.emit("jobs");
      this.emit("jobStatus");
    } else if (change.statusChanged) {
      this.emit("jobStatus");
    }
    this.emit("job", event.job);
  }

  /** Jobs added together: the `jobs-added` event, and the enqueue reply (the same batch). */
  addJobs(batch: JobsAdded<Job>) {
    const result = this.jobs.applyBatch(batch);
    if (result.added.length > 0) this.emit("jobs");
    if (result.added.length > 0 || result.statusChanged) this.emit("jobStatus");
    for (const id of result.updated) this.emit("job", this.jobs.get(id));
  }

  removeJob(event: JobRemoved) {
    if (!this.jobs.remove(event)) return;
    this.emit("jobs");
    this.emit("jobStatus");
  }

  async refreshHistory() {
    this.history = await api.listHistory();
    this.emit("history");
  }

  /** Applies a settings change right away and saves it shortly after. */
  updateSettings(patch: Partial<Settings>, onError: (message: string) => void) {
    this.onSaveError = onError;
    this.settings = { ...this.settings, ...patch };
    this.revision += 1;
    this.emit("settings");
    window.clearTimeout(this.saveTimer);
    this.saveTimer = window.setTimeout(() => void this.flushSettings(), 350);
  }

  /** Saves any pending edit now. Resolves once every queued save has finished. */
  flushSettings(): Promise<void> {
    window.clearTimeout(this.saveTimer);
    this.saveTimer = undefined;
    this.saveChain = this.saveChain.then(() => this.saveOnce());
    return this.saveChain;
  }

  private async saveOnce() {
    if (this.sentRevision === this.revision) return; // nothing new since the last attempt
    const revision = this.revision;
    const sent = this.settings;
    this.sentRevision = revision;
    try {
      const result = await api.saveSettings(sent);
      this.confirmed = result.settings;
      // The backend's copy may be normalised (trimmed, clamped); take it only when
      // nothing was edited while this save was in flight.
      if (this.revision === revision) {
        this.settings = result.settings;
        this.emit("settings");
      }
      if (result.warning && result.warning !== this.lastWarning) this.onSaveError(result.warning);
      this.lastWarning = result.warning;
    } catch (error) {
      // Rejected: put back the fields this request changed, unless they were edited
      // again since (those edits go out with the next save).
      const reverted: Partial<Record<keyof Settings, unknown>> = {};
      for (const key of Object.keys(sent) as (keyof Settings)[]) {
        if (!sameValue(sent[key], this.confirmed[key]) && sameValue(this.settings[key], sent[key])) {
          reverted[key] = this.confirmed[key];
        }
      }
      if (Object.keys(reverted).length > 0) {
        this.settings = { ...this.settings, ...(reverted as Partial<Settings>) };
        this.emit("settings");
        this.emit("settingsReverted");
      }
      this.onSaveError(errorMessage(error));
    }
  }

  /** Replaces any batch with these links and starts looking them up (never downloading). */
  startBatch(parsed: ParsedInput) {
    this.previews.clear(); // the list replaces any single link's details
    this.batch = { parsed, choices: { mode: "video", maxHeight: null, videoPreset: "compatible", audioFormat: "mp3" }, estimate: null };
    this.links.start(parsed.links);
    this.emit("batch");
  }

  clearBatch() {
    if (!this.batch) return;
    this.links.clear();
    this.batch = null;
    this.emit("batch");
  }

  /** Sends a link, or a list of links, pasted anywhere to the Download view. */
  pasteLink(url: string) {
    this.pastedLink = url;
    if (this.route === "download") this.emit("paste");
    else this.go("download");
  }

  takePastedLink(): string | null {
    const url = this.pastedLink;
    this.pastedLink = null;
    return url;
  }

  setTools(tools: ToolsStatus) {
    this.tools = tools;
    this.emit("tools");
  }

  setToolProgress(progress: ToolProgress) {
    this.toolProgress[progress.tool] = progress;
    this.emit("toolProgress", progress);
  }

  /** Installs queue (the backend refuses a second one), so none is dropped. */
  runTool(tool: "ytdlp" | "ffmpeg", action: () => Promise<ToolsStatus>, options: { quiet?: boolean } = {}): Promise<void> {
    if (this.toolPending.has(tool)) return this.toolChain;
    this.toolPending.add(tool);
    this.toolChain = this.toolChain.then(() => this.runToolNow(tool, action, options));
    return this.toolChain;
  }

  private toolChain: Promise<void> = Promise.resolve();
  private toolPending = new Set<"ytdlp" | "ffmpeg">();

  private async runToolNow(tool: "ytdlp" | "ffmpeg", action: () => Promise<ToolsStatus>, options: { quiet?: boolean }) {
    this.toolBusy[tool] = true;
    delete this.toolError[tool];
    delete this.toolProgress[tool];
    this.emit("tools");
    try {
      this.setTools(await action());
    } catch (error) {
      // A quiet run (the background migration) just tries again another time.
      if (!options.quiet) this.toolError[tool] = errorMessage(error);
    } finally {
      this.toolBusy[tool] = false;
      this.toolPending.delete(tool);
      delete this.toolProgress[tool];
      this.emit("tools");
    }
  }
}

export const store = new Store();
