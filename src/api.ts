import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { DownloadEstimate } from "./download-estimates.js";
import type { DragOutcome } from "./ui/drag-gesture.js";
import type { JobEvent, JobRemoved, JobsAdded } from "./queue-events.js";
import type { Resolution, VideoPreset } from "./resolution.js";
import type { KeptRecord, KeptRepairReport, KeptRevealResult } from "./kept-files.js";
import type { TranscriptFormat } from "./transcript-options.js";

export type { DownloadEstimate, DragOutcome, JobEvent, JobRemoved, JobsAdded, KeptRecord, KeptRepairReport, KeptRevealResult, Resolution, TranscriptFormat, VideoPreset };

export type Theme = "system" | "light" | "dark";
export type Backdrop = "glass" | "acrylic" | "mica" | "tabbed" | "none";
export type CookieKind = "none" | "browser" | "file";

export type AudioSelection = "best" | "preferMatching";

export interface Settings {
  /** Format version of these settings; the backend migrates older ones. */
  schemaVersion: number;
  outputDir: string;
  filenameTemplate: string;
  cookieSource: { kind: CookieKind; value: string };
  theme: Theme;
  glassIntensity: number;
  backdrop: Backdrop;
  maxConcurrent: number;
  /** Downloads that may be processing without holding a transfer slot. */
  maxProcessing: number;
  /** Speed limit for each download in KiB/s; 0 is no limit. */
  speedLimitKib: number;
  embedMetadata: boolean;
  /** System-wide Quick Open shortcut, e.g. "ctrl+alt+d", or "off". */
  quickOpenShortcut: QuickOpenShortcut;
  /** Closing the window leaves Claws in the notification area. */
  keepRunning: boolean;
  openAtLogin: boolean;
  /** Audio-only downloads: prefer a stream already in the chosen format. */
  audioSelection: AudioSelection;
}

export type QuickOpenShortcut = "ctrl+alt+d" | "ctrl+alt+space" | "ctrl+shift+space" | "off";

export type JobStatus =
  | "queued"
  | "downloading"
  | "processing"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export type DownloadMode = "video" | "audio" | "transcript";

export interface DownloadOptions {
  /** "transcript" saves text made from one of the site's caption tracks; no media. */
  mode: DownloadMode;
  /** Largest video to fetch, as its shorter side (yt-dlp's `res`); null is the best. */
  maxHeight: number | null;
  /**
   * Which streams a video keeps and what file they become. Jobs saved before presets read
   * as "compatible" (the backend fills it in). Ignored for audio.
   */
  videoPreset: VideoPreset;
  audioFormat: string;
  subtitleLangs: string[];
  autoSubs: boolean;
  embedSubs: boolean;
  /**
   * Skips the optional passes: no metadata, no embedded subtitles (they stay separate files)
   * and no embedded cover art in MP3 and M4A. Audio conversion, video merging and container
   * repair still happen.
   */
  fastSave: boolean;
  /**
   * What a transcript is saved as. For a transcript, `subtitleLangs` holds its one caption
   * language and `autoSubs` says whether that track is an automatic one. Jobs saved before
   * transcripts read as "txt" (the backend fills it in); ignored for video and audio.
   */
  transcriptFormat: TranscriptFormat;
}

export interface Job {
  matchOrigin?: MatchOrigin | null;
  id: string;
  url: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  duration: number | null;
  options: DownloadOptions;
  status: JobStatus;
  /** 0..1, weighted by bytes; only meaningful when `progressKnown`. 1 only once saved. */
  progress: number;
  /** False while a stream's size is unknown: show bytes and speed, not a percentage. */
  progressKnown?: boolean;
  /** Some size is an estimate, so the percentage and time left are approximate. */
  progressEstimated?: boolean;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
  speed: number | null;
  eta: number | null;
  filePath: string | null;
  error: string | null;
  createdAt: number;
  /** Settings bound when the job first started; later starts reuse them. */
  execution?: JobExecution | null;
  workspace?: { schemaVersion: number; path: string } | null;
  cancelRequested?: boolean;
  /** Increases with every change; older or repeated events are ignored. */
  revision: number;
}

export interface JobExecution {
  schemaVersion: number;
  outputDir: string;
  filenameTemplate: string;
  embedMetadata: boolean;
  audioSelection: AudioSelection;
  fragmentConcurrency: number;
  transport: "native" | "aria2c";
}

export interface HistoryItem {
  matchOrigin?: MatchOrigin | null;
  id: string;
  title: string;
  url: string;
  uploader: string | null;
  duration: number | null;
  thumbnail: string | null;
  thumbnailLocal: string | null;
  filePath: string | null;
  fileSize: number | null;
  mode: DownloadMode;
  quality: string;
  finishedAt: number;
  /**
   * What proves `filePath` is the very file this download saved. Absent (or null) for
   * entries saved by older versions: Claws never deletes such a file automatically.
   */
  fileIdentity?: FileIdentity | null;
  /** Side files the download published next to the media; deleted with it, only if recorded. */
  publishedSidecars?: PublishedFile[];
  /**
   * Subtitle files kept in the download's work folder because their names were taken in the
   * output folder. Saved with the entry, so they stay reachable after the Queue row is gone.
   * Never deleted with the entry.
   */
  keptFiles?: KeptRecord | null;
}

export type FileIdentity =
  | { kind: "fileId"; volume: number; id: string }
  | { kind: "content"; size: number; sha256: string };

export interface PublishedFile {
  relative: string;
  destination: string;
  identity: FileIdentity;
  state: "moving" | "done" | "withheld";
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
  /** "Deno" or "Node.js", the JavaScript runtime yt-dlp uses for sites like YouTube. */
  jsRuntime: string | null;
}

export interface SubtitleLang {
  code: string;
  name: string;
  auto: boolean;
}

export interface EntrySummary {
  url: string;
  /** Set when the entry had no link of its own; `url` is then the playlist's. */
  playlistIndex: number | null;
  title: string;
  duration: number | null;
  thumbnail: string | null;
  uploader: string | null;
}

export interface ProbeResult {
  audioAvailable?: boolean | null;
  kind: "video" | "playlist";
  url: string;
  title: string;
  uploader: string | null;
  thumbnail: string | null;
  duration: number | null;
  site: string | null;
  /**
   * Shorter sides of `resolutions`, largest first. In links saved before `resolutions`
   * existed these are raw format heights (the longer side for portrait video), so they are
   * offered only as caps (see resolution.ts qualityChoices).
   */
  heights: number[];
  /** Every picture size offered, largest first. Absent in links saved before it existed. */
  resolutions?: Resolution[];
  subtitles: SubtitleLang[];
  /**
   * Caption tracks a transcript can be made from, human and automatic apart (a language can
   * be in both). Absent in links saved before transcripts; empty when there are none.
   */
  captions?: SubtitleLang[] | null;
  entries: EntrySummary[];
}

export interface CachedLink {
  savedAt: number;
  result: ProbeResult;
}

export interface EnqueueItem {
  matchOrigin?: MatchOrigin | null;
  url: string;
  playlistIndex?: number | null;
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
  notices: string[];
}

export interface SavedSettings {
  settings: Settings;
  warning: string | null;
}

export interface SpotifyTrack {
  url: string;
  id: string;
  title: string | null;
  artists: string[];
  duration: number | null;
  artwork: string | null;
}

export interface SpotifyResolution {
  track: SpotifyTrack;
  warnings: string[];
}

/** What `resolve_spotify_track` rejects with; other commands reject with plain text. */
export interface SpotifyLookupIssue {
  code: string;
  message: string;
  retryable: boolean;
}

export interface YouTubeCandidate {
  url: string;
  videoId: string;
  title: string;
  uploader: string | null;
  duration: number | null;
  thumbnail: string | null;
  score: number;
  warnings: string[];
}

export interface MatchOrigin {
  kind: "spotifyYouTube";
  spotifyUrl: string;
  spotifyTitle: string | null;
  spotifyArtists: string[];
}

export const api = {
  resolveSpotify: (url: string) => invoke<SpotifyResolution>("resolve_spotify_track", { url }),
  searchSpotify: (track: SpotifyTrack, query: string) => invoke<YouTubeCandidate[]>("search_spotify_matches", { track, query }),
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  appReady: () => invoke<void>("app_ready"),
  quit: () => invoke<void>("quit_app"),
  probe: (url: string) => invoke<ProbeResult>("probe_url", { url }),
  cachedLink: (url: string) => invoke<CachedLink | null>("cached_link", { url }),
  /**
   * Download size and free space needed for one item with these options (estimate.rs).
   * Display only: the download still checks the actual free space when it starts.
   */
  estimate: (url: string, playlistIndex: number | null, options: DownloadOptions) =>
    invoke<DownloadEstimate>("estimate_download", { url, playlistIndex, options }),
  /** `source` is the link that was looked up, saved so its next lookup is instant. */
  enqueue: (items: EnqueueItem[], options: DownloadOptions, source: string | null) =>
    invoke<JobsAdded<Job>>("enqueue", { items, options, source }),
  pause: (id: string) => invoke<void>("pause_job", { id }),
  resume: (id: string) => invoke<void>("resume_job", { id }),
  cancel: (id: string) => invoke<void>("cancel_job", { id }),
  remove: (id: string) => invoke<void>("remove_job", { id }),
  clearFinished: () => invoke<void>("clear_finished"),
  saveSettings: (settings: Settings) => invoke<SavedSettings>("save_settings", { settings }),
  listHistory: () => invoke<HistoryItem[]>("list_history"),
  openDownload: (id: string) => invoke<void>("open_download", { id }),
  revealDownload: (id: string) => invoke<void>("reveal_download", { id }),
  dragDownload: (id: string) => invoke<DragOutcome>("drag_download", { id }),
  /** Opens the folder holding the kept subtitle files; says when it is offline, gone or unproven. */
  showKeptFiles: (id: string) => invoke<KeptRevealResult>("show_kept_files", { id }),
  /** Looks again for kept files of older downloads (also in folders that could not be read before). */
  repairKeptFiles: () => invoke<KeptRepairReport>("repair_kept_files"),
  keptFilesReport: () => invoke<KeptRepairReport | null>("kept_files_report"),
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
