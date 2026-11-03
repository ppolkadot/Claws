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

type Topic = "route" | "jobs" | "job" | "history" | "settings" | "tools" | "toolProgress";
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
};

class Store {
  route: Route = "download";
  settings: Settings = defaultSettings;
  jobs = new Map<string, Job>();
  history: HistoryItem[] = [];
  tools: ToolsStatus = { ytdlp: null, ffmpeg: null, latestYtdlp: null };
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
    this.jobs = new Map(data.jobs.map((job) => [job.id, job]));
    this.history = data.history;
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
    this.settings = { ...this.settings, ...patch };
    this.emit("settings");
    window.clearTimeout(this.saveTimer);
    this.saveTimer = window.setTimeout(async () => {
      try {
        this.settings = await api.saveSettings(this.settings);
      } catch (error) {
        onError(errorMessage(error));
      }
    }, 350);
  }

  setTools(tools: ToolsStatus) {
    this.tools = tools;
    this.emit("tools");
  }

  setToolProgress(progress: ToolProgress) {
    this.toolProgress[progress.tool] = progress;
    this.emit("toolProgress", progress);
  }

  /** Runs an install or update for one tool and tracks its busy and error state. */
  async runTool(tool: "ytdlp" | "ffmpeg", action: () => Promise<ToolsStatus>) {
    this.toolBusy[tool] = true;
    delete this.toolError[tool];
    delete this.toolProgress[tool];
    this.emit("tools");
    try {
      this.setTools(await action());
    } catch (error) {
      this.toolError[tool] = errorMessage(error);
    } finally {
      this.toolBusy[tool] = false;
      delete this.toolProgress[tool];
      this.emit("tools");
    }
  }
}

export const store = new Store();
