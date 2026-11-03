import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Theme = "system" | "light" | "dark";
export type Backdrop = "glass" | "acrylic" | "mica" | "tabbed" | "none";
export type CookieKind = "none" | "browser" | "file";

export interface Settings {
  outputDir: string;
  filenameTemplate: string;
  cookieSource: { kind: CookieKind; value: string };
  theme: Theme;
  glassIntensity: number;
  backdrop: Backdrop;
  maxConcurrent: number;
  embedMetadata: boolean;
}

export type JobStatus =
  | "queued"
  | "downloading"
  | "processing"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export interface DownloadOptions {
  mode: "video" | "audio";
  maxHeight: number | null;
  audioFormat: string;
  subtitleLangs: string[];
  autoSubs: boolean;
  embedSubs: boolean;
}

export interface Job {
  id: string;
  url: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
  options: DownloadOptions;
  status: JobStatus;
  progress: number;
  speed: number | null;
  eta: number | null;
  filePath: string | null;
  error: string | null;
  createdAt: number;
}

export interface HistoryItem {
  id: string;
  title: string;
  url: string;
  uploader: string | null;
  duration: number | null;
  thumbnail: string | null;
  thumbnailLocal: string | null;
  filePath: string | null;
  fileSize: number | null;
  mode: "video" | "audio";
  quality: string;
  finishedAt: number;
}

export interface ToolInfo {
  path: string;
  version: string;
  managed: boolean;
  /** yt-dlp only: the faster-starting folder build. */
  fastStart: boolean;
}

export interface ToolsStatus {
  ytdlp: ToolInfo | null;
  ffmpeg: ToolInfo | null;
  latestYtdlp: string | null;
}

export interface SubtitleLang {
  code: string;
  name: string;
  auto: boolean;
}

export interface EntrySummary {
  url: string;
  title: string;
  duration: number | null;
  thumbnail: string | null;
  uploader: string | null;
}

export interface ProbeResult {
  kind: "video" | "playlist";
  url: string;
  title: string;
  uploader: string | null;
  thumbnail: string | null;
  duration: number | null;
  site: string | null;
  heights: number[];
  subtitles: SubtitleLang[];
  entries: EntrySummary[];
}

export interface EnqueueItem {
  url: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
}

export interface ToolProgress {
  tool: "ytdlp" | "ffmpeg";
  received: number;
  total: number;
  stage: "download" | "extract";
}

interface Bootstrap {
  settings: Settings;
  jobs: Job[];
  history: HistoryItem[];
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  appReady: () => invoke<void>("app_ready"),
  probe: (url: string) => invoke<ProbeResult>("probe_url", { url }),
  enqueue: (items: EnqueueItem[], options: DownloadOptions) =>
    invoke<Job[]>("enqueue", { items, options }),
  pause: (id: string) => invoke<void>("pause_job", { id }),
  resume: (id: string) => invoke<void>("resume_job", { id }),
  cancel: (id: string) => invoke<void>("cancel_job", { id }),
  remove: (id: string) => invoke<void>("remove_job", { id }),
  clearFinished: () => invoke<void>("clear_finished"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  listHistory: () => invoke<HistoryItem[]>("list_history"),
  openDownload: (id: string) => invoke<void>("open_download", { id }),
  revealDownload: (id: string) => invoke<void>("reveal_download", { id }),
  openOutputDir: () => invoke<void>("open_output_dir"),
  deleteHistory: (id: string, deleteFile: boolean) =>
    invoke<void>("delete_history", { id, deleteFile }),
  clearHistory: () => invoke<void>("clear_history"),
  toolsStatus: () => invoke<ToolsStatus>("tools_status"),
  installYtdlp: () => invoke<ToolsStatus>("install_ytdlp"),
  updateYtdlp: () => invoke<ToolsStatus>("update_ytdlp"),
  installFfmpeg: () => invoke<ToolsStatus>("install_ffmpeg"),
  openFontPage: () => invoke<void>("open_font_page"),
};

export function onEvent<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (event) => handler(event.payload));
}

/** Errors from Rust arrive as plain strings. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}
