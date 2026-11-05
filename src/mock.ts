// Development only: lets the interface run in a plain browser (`npm run dev`) without
// the Rust backend. Every title, number and image below is synthetic sample data.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { DownloadOptions, EnqueueItem, HistoryItem, Job, ProbeResult, Settings } from "./api";

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
          url: `https://example.com/v/${i}`,
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
        title: "Sample: Coastal timelapse, from sunrise to the last light",
        uploader: "Sample Channel",
        thumbnail: art(205, 265),
        duration: 754,
        site: "Sample Site",
        heights: [2160, 1440, 1080, 720, 480, 360, 144],
        subtitles: [
          { code: "en", name: "English", auto: false },
          { code: "es", name: "Spanish", auto: false },
          { code: "fr", name: "French (auto)", auto: true },
        ],
        entries: [],
      };

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
          return probe(String(payload.url));
        case "cached_link":
          return null;
        case "enqueue":
          return {
            schemaVersion: 1,
            jobs: (payload.items as EnqueueItem[]).map((item, i) => {
              const added = job(`n${Date.now()}${i}`, item.title, [200, 260], { createdAt: Date.now() + i });
              return { schemaVersion: 1, jobId: added.id, revision: added.revision, job: added };
            }),
          };
        case "save_settings":
          return { settings: payload.settings, warning: null };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
