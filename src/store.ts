import {
  api,
  errorMessage,
  type HistoryItem,
  type Job,
  type Settings,
  type ToolProgress,
  type ToolsStatus,
} from "./api";

export type Route = "download" | "queue" | "library" | "settings";

type Topic =
  | "route"
  | "jobs"
  | "job"
  | "history"
  | "settings"
  /** A save was rejected and some fields went back to their saved values. */
  | "settingsReverted"
  /** A link was pasted somewhere in the window; see `takePastedLink`. */
  | "paste"
  /** Quick Open without a link: put the cursor in the link field. */
  | "focusLink"
  | "tools"
  | "toolProgress";

const sameValue = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
type Listener = (detail?: unknown) => void;

const defaultSettings: Settings = {
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
};

class Store {
  route: Route = "download";
  settings: Settings = defaultSettings;
  jobs = new Map<string, Job>();
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

  on(topic: Topic, listener: Listener): () => void {
    let set = this.listeners.get(topic);
    if (!set) this.listeners.set(topic, (set = new Set()));
    set.add(listener);
    return () => set.delete(listener);
  }

  emit(topic: Topic, detail?: unknown) {
    this.listeners.get(topic)?.forEach((listener) => listener(detail));
  }

  jobList(): Job[] {
    return [...this.jobs.values()].sort((a, b) => a.createdAt - b.createdAt);
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
    this.jobs = new Map(data.jobs.map((job) => [job.id, job]));
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

  upsertJob(job: Job) {
    this.jobs.set(job.id, job);
    this.emit("job", job);
    this.emit("jobs");
  }

  removeJob(id: string) {
    this.jobs.delete(id);
    this.emit("jobs");
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

  /** Sends a link pasted anywhere to the Download view. */
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
