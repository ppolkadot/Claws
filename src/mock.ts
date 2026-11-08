// Development only: lets the interface run in a plain browser (`npm run dev`) without
// the Rust backend. Every title, number and image below is synthetic sample data.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { DownloadOptions, EnqueueItem, HistoryItem, Job, JobEvent, JobRemoved, JobsAdded, ProbeResult, Settings } from "./api";
import { ALREADY_QUEUED } from "./download-input";

function art(hueA: number, hueB: number): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360" viewBox="0 0 640 360">
    <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="hsl(${hueA} 80% 58%)"/><stop offset="1" stop-color="hsl(${hueB} 75% 38%)"/>
    </linearGradient></defs>
    <rect width="640" height="360" fill="url(#g)"/>
    <circle cx="470" cy="110" r="120" fill="hsl(${hueB} 90% 70%)" opacity=".35"/>
    <circle cx="150" cy="300" r="150" fill="hsl(${hueA} 90% 80%)" opacity=".22"/>
  </svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

const now = Date.now();
const defaults: DownloadOptions = { mode: "video", maxHeight: 1080, audioFormat: "mp3", subtitleLangs: [], autoSubs: false, embedSubs: false, fastSave: false };

const settings: Settings = {
  schemaVersion: 2,
  outputDir: "C:\\Users\\Sample\\Downloads\\Claws",
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

const job = (id: string, title: string, hues: [number, number], over: Partial<Job>): Job => ({
  id,
  url: `https://example.com/watch/${id}`,
  title,
  thumbnail: art(hues[0], hues[1]),
  uploader: "Sample Channel",
  duration: 754,
  options: defaults,
  status: "queued",
  progress: 0,
  speed: null,
  eta: null,
  filePath: null,
  error: null,
  createdAt: now,
  revision: 1,
  ...over,
});

const jobs: Job[] = [
  job("a1", "Sample: Coastal timelapse in 4K", [200, 250], { status: "downloading", progress: 0.62, progressKnown: true, downloadedBytes: 620_000_000, totalBytes: 1_000_000_000, speed: 6_400_000, eta: 41, createdAt: now - 5000 }),
  job("a2", "Sample: Studio session, full set", [280, 330], { status: "processing", progress: 0.99, createdAt: now - 4000, options: { ...defaults, mode: "audio", audioFormat: "mp3" } }),
  job("a3", "Sample: Interview, part two", [20, 350], { status: "paused", progress: 0.34, createdAt: now - 3000 }),
  job("a4", "Sample: A very long title that should be truncated gracefully when the window is narrow", [140, 190], { status: "failed", error: "This video is private. Sign in with cookies to download it.", createdAt: now - 2000 }),
  job("a5", "Sample: Mountain road, dawn", [330, 20], { status: "queued", createdAt: now - 1000 }),
  job("a6", "Sample: Cooking, ten minute pasta", [40, 10], { status: "completed", progress: 1, filePath: "C:\\Users\\Sample\\Downloads\\Claws\\pasta.mp4", createdAt: now }),
];

const history: HistoryItem[] = [
  ["Sample: Cooking, ten minute pasta", [40, 10], "1080p", 184_000_000, 596],
  ["Sample: City lights, night walk", [250, 300], "4K", 912_000_000, 1310],
  ["Sample: Piano improvisation no. 4", [300, 220], "MP3", 9_400_000, 402],
  ["Sample: Understanding tides", [190, 230], "720p", 96_000_000, 845],
  ["Sample: Garden tour in spring", [110, 160], "1080p", 241_000_000, 1033],
  ["Sample: Short film, silent", [350, 300], "1440p", 502_000_000, 1620],
].map(([title, hues, quality, size, duration], i) => ({
  id: `h${i}`,
  title: title as string,
  url: `https://example.com/v/${i}`,
  uploader: "Sample Channel",
  duration: duration as number,
  thumbnail: art((hues as number[])[0], (hues as number[])[1]),
  thumbnailLocal: null,
  filePath: `C:\\Users\\Sample\\Downloads\\Claws\\sample-${i}.mp4`,
  fileSize: size as number,
  mode: quality === "MP3" ? "audio" : "video",
  quality: quality as string,
  finishedAt: now - i * 3_600_000 * 7,
}));

/** A stable number from a link, so each sample link behaves the same every time. */
function hash(text: string): number {
  let value = 2166136261;
  for (let i = 0; i < text.length; i++) {
    value ^= text.charCodeAt(i);
    value = Math.imul(value, 16777619);
  }
  return value >>> 0;
}

const sampleTitles = [
  "Sample: Coastal timelapse, from sunrise to the last light",
  "Sample: Night train through the mountains",
  "Sample: Ceramics, throwing a tall vase",
  "Sample: Rain on a tin roof, one hour",
  "Sample: Street food market tour",
  "Sample: Lecture on tidal energy",
];

const probe = (url: string): ProbeResult =>
  url.includes("list")
    ? {
        kind: "playlist",
        url,
        title: "Sample: Weekend travel diaries",
        uploader: "Sample Channel",
        thumbnail: null,
        duration: null,
        site: "Sample Site",
        heights: [],
        subtitles: [],
        entries: Array.from({ length: 8 }, (_, i) => ({
          url: `https://example.com/v/${hash(url) % 1000}-${i}`,
          playlistIndex: null,
          title: `Sample: Travel diary, day ${i + 1}`,
          duration: 420 + i * 61,
          thumbnail: art(20 + i * 40, 60 + i * 40),
          uploader: "Sample Channel",
        })),
      }
    : {
        kind: "video",
        url,
        title: sampleTitles[hash(url) % sampleTitles.length],
        uploader: "Sample Channel",
        thumbnail: art(hash(url) % 360, (hash(url) + 60) % 360),
        duration: 120 + (hash(url) % 1800),
        site: "Sample Site",
        heights: [2160, 1440, 1080, 720, 480, 360, 144],
        subtitles: [
          { code: "en", name: "English", auto: false },
          { code: "es", name: "Spanish", auto: false },
          { code: "fr", name: "French (auto)", auto: true },
        ],
        entries: [],
      };

/**
 * Lookups take 0.3 to 2.3 seconds, chosen by the link, so a batch answers out of order.
 * A link containing "fail" fails, "private" fails as a private video, "list" is a playlist.
 */
async function probeLink(url: string): Promise<ProbeResult> {
  // scripts/dev/batch-bench.mjs shortens the delays with window.__clawsMockProbeScale.
  const scale = (window as unknown as { __clawsMockProbeScale?: number }).__clawsMockProbeScale ?? 1;
  await new Promise((resolve) => setTimeout(resolve, (300 + (hash(url) % 2000)) * scale));
  if (url.includes("private")) throw "This video is private. Sign in with cookies to download it.";
  if (url.includes("fail")) throw "Sample failure: the site did not answer. Try again later.";
  return probe(url);
}

// Every job the mock knows, so duplicate checks, cancel and remove behave as the backend's.
const mockJobs = new Map(jobs.map((j) => [j.id, j]));
/** The backend keeps a job's playlist position; the interface's Job type does not. */
const playlistIndexOf = new Map<string, number | null>();
let mockRevision = 100;
let mockJobId = 0;

// Keyed like the backend's duplicate check (downloader.rs enqueue).
const queueKey = (url: string, playlistIndex: number | null | undefined, options: DownloadOptions) =>
  JSON.stringify([url, playlistIndex && playlistIndex > 0 ? playlistIndex : null, options]);

/** Validates, de-duplicates and announces the batch the way the backend does. */
function enqueue(items: EnqueueItem[], options: DownloadOptions): JobsAdded<Job> {
  if (items.length === 0) throw "Nothing to download.";
  if (items.length > 500) throw "Too many items at once (limit 500).";
  // Built fresh for each call from jobs not finished, cancelled or failed, as the backend
  // does; a refused call changes nothing.
  const pending = new Set(
    [...mockJobs.values()].filter((j) => !["completed", "cancelled", "failed"].includes(j.status)).map((j) => queueKey(j.url, playlistIndexOf.get(j.id), j.options)),
  );
  const seen = new Set<string>();
  const created: Job[] = [];
  const createdIndex: (number | null)[] = [];
  for (const item of items) {
    if (!/^https?:\/\/[^/\s]+/i.test(item.url)) throw "Only http and https links are supported.";
    const key = queueKey(item.url, item.playlistIndex, options);
    if (pending.has(key) || seen.has(key)) continue;
    seen.add(key);
    createdIndex.push(item.playlistIndex && item.playlistIndex > 0 ? item.playlistIndex : null);
    const hue = hash(item.url) % 360;
    created.push(
      job(`n${++mockJobId}`, item.title.trim() || "Untitled", [hue, (hue + 50) % 360], {
        url: item.url,
        thumbnail: item.thumbnail ?? art(hue, (hue + 50) % 360),
        uploader: item.uploader,
        duration: item.duration,
        options,
        createdAt: Date.now() + created.length,
        revision: ++mockRevision,
      }),
    );
  }
  if (created.length === 0) throw ALREADY_QUEUED;
  created.forEach((added, i) => {
    mockJobs.set(added.id, added);
    playlistIndexOf.set(added.id, createdIndex[i]);
  });
  const batch: JobsAdded<Job> = { schemaVersion: 1, jobs: created.map((added) => ({ schemaVersion: 1, jobId: added.id, revision: added.revision, job: added })) };
  void emit("jobs-added", batch);
  return batch;
}

function cancelJob(id: string) {
  const current = mockJobs.get(id);
  if (!current || ["completed", "cancelled", "failed"].includes(current.status)) return;
  const next: Job = { ...current, status: "cancelled", speed: null, eta: null, revision: ++mockRevision };
  mockJobs.set(id, next);
  const event: JobEvent<Job> = { schemaVersion: 1, jobId: id, revision: next.revision, job: next };
  void emit("job-update", event);
}

function removeJobs(ids: string[]) {
  for (const id of ids) {
    if (!mockJobs.delete(id)) continue;
    playlistIndexOf.delete(id);
    const event: JobRemoved = { schemaVersion: 1, jobId: id, revision: ++mockRevision };
    void emit("job-removed", event);
  }
}

export function installMock() {
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      const payload = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "bootstrap":
          return { settings, jobs, history, notices: [] };
        case "tools_status":
          return {
              ytdlp: { path: "C:\\sample\\yt-dlp\\yt-dlp.exe", version: "2026.10.26", managed: true, fastStart: true },
              ffmpeg: { path: "C:\\sample\\ffmpeg.exe", version: "8.0", managed: false, fastStart: false },
              latestYtdlp: null,
              jsRuntime: "Deno",
          };
        case "list_history":
          return history;
        case "probe_url":
          return probeLink(String(payload.url));
        case "cached_link":
          return null;
        case "enqueue":
          return enqueue(payload.items as EnqueueItem[], payload.options as DownloadOptions);
        case "cancel_job":
          return cancelJob(String(payload.id));
        case "remove_job":
          return removeJobs([String(payload.id)]);
        case "clear_finished":
          return removeJobs([...mockJobs.values()].filter((j) => j.status === "completed" || j.status === "cancelled").map((j) => j.id));
        case "save_settings":
          return { settings: payload.settings, warning: null };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
