// Development only: lets the interface run in a plain browser (`npm run dev`) without
// the Rust backend. Every title, number and image below is synthetic sample data.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { DownloadEstimate, DownloadOptions, EnqueueItem, HistoryItem, Job, JobEvent, JobRemoved, JobsAdded, ProbeResult, Resolution, Settings } from "./api";
import { ALREADY_QUEUED } from "./download-input";
import { transcriptFileSuffix, transcriptText } from "./transcript-options";

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
const defaults: DownloadOptions = { mode: "video", maxHeight: 1080, videoPreset: "compatible", audioFormat: "mp3", subtitleLangs: [], autoSubs: false, embedSubs: false, fastSave: false, transcriptFormat: "txt" };

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
  job("a5", "Sample: Mountain road, dawn", [330, 20], { status: "queued", createdAt: now - 1000, options: { ...defaults, maxHeight: 4320, videoPreset: "original" } }),
  job("a6", "Sample: Cooking, ten minute pasta", [40, 10], { status: "completed", progress: 1, filePath: "C:\\Users\\Sample\\Downloads\\Claws\\pasta.mp4", createdAt: now }),
];

const history: HistoryItem[] = [
  ["Sample: Cooking, ten minute pasta", [40, 10], "1080p", 184_000_000, 596],
  ["Sample: City lights, night walk", [250, 300], "4K", 912_000_000, 1310],
  ["Sample: Glacier flyover", [180, 210], "4320p · Original", 2_480_000_000, 380],
  ["Sample: Piano improvisation no. 4", [300, 220], "MP3", 9_400_000, 402],
  ["Sample: Understanding tides", [190, 230], "720p", 96_000_000, 845],
  ["Sample: Lecture on tidal energy", [200, 160], "Transcript · English · TXT", 18_400, 2410],
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
  filePath: `C:\\Users\\Sample\\Downloads\\Claws\\sample-${i}.${(quality as string).startsWith("Transcript") ? "txt" : "mp4"}`,
  fileSize: size as number,
  mode: quality === "MP3" ? "audio" : (quality as string).startsWith("Transcript") ? "transcript" : "video",
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

/**
 * Picture sizes for a sample link, as probe.rs reports them: "portrait" links are vertical
 * video, "hd" links stop at 1080p, and the rest reach 8K, with a 5120×2880 step and a 4K stream
 * that also comes in HDR.
 */
function sampleResolutions(url: string): Resolution[] {
  const size = (width: number, height: number, vcodec: string, extra: Partial<Resolution> = {}): Resolution => ({
    shortSide: Math.min(width, height),
    width,
    height,
    vcodec,
    dynamicRange: "SDR",
    fps: 30,
    ...extra,
  });
  if (url.includes("portrait")) return [size(1080, 1920, "avc1.640028"), size(720, 1280, "avc1.4d401f"), size(360, 640, "avc1.4d401e")];
  const ladder = [
    size(7680, 4320, "av01.0.17M.08"),
    size(5120, 2880, "vp09.00.61.08"),
    size(3840, 2160, "vp09.02.51.10", { dynamicRange: "HDR10", fps: 60 }),
    size(2560, 1440, "vp09.00.50.08"),
    size(1920, 1080, "avc1.640028"),
    size(1280, 720, "avc1.4d401f"),
    size(854, 480, "avc1.4d401e"),
    size(640, 360, "avc1.4d401e"),
    size(256, 144, "avc1.4d400c"),
  ];
  return url.includes("hd") ? ladder.filter((r) => r.shortSide <= 1080) : ladder;
}

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
        audioAvailable: true,
        url,
        title: sampleTitles[hash(url) % sampleTitles.length],
        uploader: "Sample Channel",
        thumbnail: art(hash(url) % 360, (hash(url) + 60) % 360),
        duration: 120 + (hash(url) % 1800),
        site: "Sample Site",
        heights: sampleResolutions(url).map((r) => r.shortSide),
        resolutions: sampleResolutions(url),
        ...sampleCaptions(url),
        entries: [],
      };

/**
 * Caption tracks as probe.rs reports them: links containing "silent" have none (the
 * Transcript type is then unavailable); others have human English and Spanish plus the
 * speech-recognition track (`-orig`). YouTube's machine-translated automatic tracks are
 * not offered for transcripts.
 */
function sampleCaptions(url: string): Pick<ProbeResult, "subtitles" | "captions"> {
  if (url.includes("silent")) return { subtitles: [], captions: [] };
  return {
    subtitles: [
      { code: "en", name: "English", auto: false },
      { code: "es", name: "Spanish", auto: false },
      { code: "fr", name: "French (auto)", auto: true },
    ],
    captions: [
      { code: "en", name: "English", auto: false },
      { code: "es", name: "Spanish", auto: false },
      { code: "en-orig", name: "English (Original)", auto: true },
    ],
  };
}

/**
 * Lookups take 0.3 to 2.3 seconds, chosen by the link, so a batch answers out of order.
 * A link containing "fail" fails, "private" fails as a private video, "list" is a playlist,
 * "silent" has no captions.
 */
async function probeLink(url: string): Promise<ProbeResult> {
  // scripts/dev/batch-bench.mjs shortens the delays with window.__clawsMockProbeScale.
  const scale = (window as unknown as { __clawsMockProbeScale?: number }).__clawsMockProbeScale ?? 1;
  await new Promise((resolve) => setTimeout(resolve, (300 + (hash(url) % 2000)) * scale));
  if (url.includes("private")) throw "This video is private. Sign in with cookies to download it.";
  if (url.includes("fail")) throw "Sample failure: the site did not answer. Try again later.";
  return probe(url);
}

const unknownEstimate = (reason: string): DownloadEstimate => ({
  downloadBytes: null,
  confidence: "unknown",
  parts: [],
  container: null,
  peakTempBytes: null,
  requiredFreeBytes: null,
  outputBytes: null,
  converted: false,
  unknownReason: reason,
});

/**
 * A size preview like estimate.rs gives: sizes from the link (so each sample link answers
 * the same every time) and the choices, with the backend's free-space formula. About one
 * link in six has an unknown size; a link containing "fail" cannot be estimated at all.
 */
async function estimateLink(url: string, playlistIndex: number | null, options: DownloadOptions): Promise<DownloadEstimate> {
  const key = hash(`${url}#${playlistIndex ?? ""}`);
  await new Promise((resolve) => setTimeout(resolve, 150 + (key % 500)));
  if (url.includes("fail")) throw "Sample failure: the site did not answer.";
  if (options.mode === "transcript") return unknownEstimate("Text only: the video is not downloaded.");
  const unknown = unknownEstimate;
  // disk_budget.rs `required`: the peak plus a margin of at least 512 MiB.
  const required = (peak: number) => peak + Math.max(512 * 1024 * 1024, Math.floor(peak / 10));
  if (key % 6 === 0) return unknown("The site does not say how large this download is.");
  const seconds = 120 + (hash(url) % 1800);
  const bytesAt = (kbps: number) => Math.round((kbps * 1000) / 8 * seconds);
  // download_policy.rs `ProcessingPolicy::rewrites_output`: metadata, and for MP3 and M4A the
  // cover art, are whole-file passes that Fast Save skips.
  const rewrite = !options.fastSave && (settings.embedMetadata || (options.mode === "audio" && (options.audioFormat === "mp3" || options.audioFormat === "m4a")));
  if (options.mode === "audio") {
    const matching = settings.audioSelection === "preferMatching" && options.audioFormat === "m4a";
    const source = matching
      ? { formatId: "a140", acodec: "mp4a.40.2", ext: "m4a", kbps: 128 }
      : { formatId: "a251", acodec: "opus", ext: "webm", kbps: 160 };
    const bytes = bytesAt(source.kbps);
    const converts = options.audioFormat !== "best";
    // disk_budget.rs `audio_output`: WAV and FLAC from the duration, the rest about the source.
    const output =
      options.audioFormat === "wav" ? Math.floor(seconds * 192_000) : options.audioFormat === "flac" ? Math.floor(seconds * (48_000 * 2 * 2 * 0.65)) : bytes;
    // disk_budget.rs `peak_bytes`: every audio target goes through extraction (even "Original"),
    // so the source stays beside the output, and a rewrite adds a second output-sized copy.
    const peak = bytes + output * (rewrite ? 2 : 1);
    return {
      downloadBytes: bytes,
      confidence: "approximate",
      parts: [{ formatId: source.formatId, vcodec: null, acodec: source.acodec, ext: source.ext, width: null, height: null, bytes, exact: false }],
      container: converts ? options.audioFormat : null,
      peakTempBytes: peak,
      requiredFreeBytes: required(peak),
      outputBytes: output,
      converted: converts,
      unknownReason: null,
    };
  }
  // The largest size within the cap (by shorter side), else the smallest above it, as
  // yt-dlp's `res` sort picks; a playlist item uses the default ladder.
  const sizes = sampleResolutions(url);
  const cap = options.maxHeight ?? Infinity;
  const chosen = sizes.find((r) => r.shortSide <= cap) ?? sizes[sizes.length - 1];
  const kbps = Math.max(100, Math.round((chosen.width! * chosen.height!) / 500));
  const video = bytesAt(kbps);
  // Compatible prefers H.264 and AAC (only up to 1080p here) and saves MP4. Original takes
  // the site's own best streams, Opus audio included, so the file is MKV.
  const original = options.videoPreset === "original";
  const vcodec = original || chosen.shortSide > 1080 ? chosen.vcodec! : "avc1.640028";
  const sound = original ? { formatId: "a251", acodec: "opus", ext: "webm", kbps: 160 } : { formatId: "a140", acodec: "mp4a.40.2", ext: "m4a", kbps: 128 };
  const audio = bytesAt(sound.kbps);
  const exact = key % 3 === 0;
  return {
    downloadBytes: video + audio,
    confidence: "approximate",
    parts: [
      { formatId: `v${chosen.shortSide}`, vcodec, acodec: null, ext: vcodec.startsWith("avc1") ? "mp4" : "webm", width: chosen.width, height: chosen.height, bytes: video, exact },
      { formatId: sound.formatId, vcodec: null, acodec: sound.acodec, ext: sound.ext, width: null, height: null, bytes: audio, exact: false },
    ],
    container: original && !(vcodec.startsWith("avc1") && sound.acodec.startsWith("mp4a")) ? "mkv" : "mp4",
    // Merged: one extra copy of both streams (disk_budget.rs).
    peakTempBytes: 2 * (video + audio),
    requiredFreeBytes: required(2 * (video + audio)),
    outputBytes: video + audio,
    converted: false,
    unknownReason: null,
  };
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
function enqueue(items: EnqueueItem[], requested: DownloadOptions): JobsAdded<Job> {
  if (items.length === 0) throw "Nothing to download.";
  if (items.length > 500) throw "Too many items at once (limit 500).";
  // DownloadOptions::validate: no editing preset yet, no video preset for audio, one caption
  // language and no media choices for a transcript.
  if (requested.videoPreset === "editingProRes") throw "The editing preset is not available yet.";
  const transcript = requested.mode === "transcript";
  if (transcript && requested.subtitleLangs.length !== 1) throw "Choose one caption language for the transcript.";
  const options: DownloadOptions = transcript
    ? { ...requested, maxHeight: null, videoPreset: "compatible", audioFormat: "mp3", embedSubs: false, fastSave: false, transcriptFormat: requested.transcriptFormat ?? "txt" }
    : {
        ...requested,
        videoPreset: requested.mode === "audio" ? "compatible" : (requested.videoPreset ?? "compatible"),
        maxHeight: requested.maxHeight == null ? null : Math.min(8640, Math.max(144, requested.maxHeight)),
        transcriptFormat: "txt",
      };
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
        matchOrigin: item.matchOrigin ?? null,
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
  if (transcript) created.forEach((added) => emulateTranscript(added.id));
  return batch;
}


function updateJob(id: string, change: Partial<Job>): Job | null {
  const current = mockJobs.get(id);
  if (!current || ["completed", "cancelled", "failed"].includes(current.status)) return null;
  const next: Job = { ...current, ...change, revision: ++mockRevision };
  mockJobs.set(id, next);
  const event: JobEvent<Job> = { schemaVersion: 1, jobId: id, revision: next.revision, job: next };
  void emit("job-update", event);
  return next;
}

/**
 * A transcript as the backend runs it: captions only, shown as processing with no byte
 * progress, then saved as a small text file and recorded in the Library.
 */
function emulateTranscript(id: string) {
  window.setTimeout(() => updateJob(id, { status: "processing", progressKnown: false }), 300);
  window.setTimeout(() => {
    const current = mockJobs.get(id);
    if (!current) return;
    const o = current.options;
    const filePath = `C:\\Users\\Sample\\Downloads\\Claws\\${current.title}${transcriptFileSuffix(o)}`;
    const done = updateJob(id, { status: "completed", progress: 1, progressKnown: true, filePath });
    if (!done) return;
    history.unshift({
      id,
      title: current.title,
      url: current.url,
      uploader: current.uploader,
      duration: current.duration,
      thumbnail: current.thumbnail,
      thumbnailLocal: null,
      filePath,
      fileSize: 12_000 + (hash(current.url) % 40_000),
      mode: "transcript",
      quality: transcriptText(o),
      finishedAt: Date.now(),
    });
    void emit("history-changed", null);
  }, 1800);
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
        case "drag_download":
          throw new Error("Native file dragging requires the Windows app.");
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
        case "kept_files_report":
          return { attached: 0, inaccessible: [], ignored: 0 };
        case "show_kept_files":
          return { status: "shown", folder: "C:\\Users\\Sample\\Downloads\\Claws\\.claws-work\\sample", files: 1 };
        case "probe_url":
          return probeLink(String(payload.url));
        case "resolve_spotify_track": {
          const input = String(payload.url);
          const id = /\/track\/([A-Za-z0-9]{22})(?:[/?#]|$)/.exec(input)?.[1];
          if (!id) throw new Error("Sample mode supports full Spotify track links only. Paste the full Spotify track link.");
          return { track: { id, url: `https://open.spotify.com/track/${id}`, title: "Sample: Midnight tide", artists: ["Sample Artist"], duration: 213.573, artwork: null }, warnings: [] };
        }
        case "search_spotify_matches":
          return ["dQw4w9WgXcQ", "abcdefghijk", "lmnopqrstuv"].map((videoId, index) => ({
            videoId, url: `https://www.youtube.com/watch?v=${videoId}`,
            title: index === 0 ? "Sample: Midnight tide — studio recording" : index === 1 ? "Sample: Midnight tide — live" : "Sample: Midnight tide — cover",
            uploader: index === 2 ? "Sample Cover Artist" : "Sample Artist", duration: 214 + index * 25,
            thumbnail: null, score: 0.94 - index * 0.2, warnings: index === 0 ? ["Similar title"] : [index === 1 ? "May be a live recording" : "May be a cover recording"],
          }));
        case "cached_link":
          return null;
        case "estimate_download":
          if ((payload.options as DownloadOptions).videoPreset === "editingProRes") throw "The editing preset is not available yet.";
          return estimateLink(String(payload.url), (payload.playlistIndex as number | null) ?? null, payload.options as DownloadOptions);
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
