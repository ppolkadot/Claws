export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function formatSpeed(bytesPerSecond: number | null | undefined): string {
  if (bytesPerSecond == null || bytesPerSecond <= 0) return "";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return "";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** A clock reading: 0:42, 1:03, 1:02:03. Unlike `formatDuration`, zero reads as 0:00. */
export function clock(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "";
  return Math.round(seconds) === 0 ? "0:00" : formatDuration(seconds);
}

/** "2.6 of 5.6 GB": both numbers in the unit the total reads best in. */
export function formatBytesOf(done: number, total: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let unit = 0;
  let scale = 1;
  while (total / scale >= 1024 && unit < units.length - 1) {
    scale *= 1024;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : 1;
  return `${(done / scale).toFixed(digits)} of ${(total / scale).toFixed(digits)} ${units[unit]}`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return "";
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))}s left`;
  const text = formatDuration(seconds);
  return `${text} left`;
}

export function formatDate(timestamp: number): string {
  const date = new Date(timestamp);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) {
    return `Today, ${date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  // Month names in the interface's language, like "Today" and "Yesterday" above.
  return date.toLocaleDateString("en", { month: "short", day: "numeric", year: "numeric" });
}

export function folderName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.slice(-2).join("\\") || path;
}

/** Display names for yt-dlp's extractor keys ("Youtube", "TwitchVod") where the key is not the brand. */
const SITE_NAMES: [string, string][] = [
  ["youtube", "YouTube"],
  ["soundcloud", "SoundCloud"],
  ["tiktok", "TikTok"],
  ["twitch", "Twitch"],
  ["twitter", "X"],
  ["vimeo", "Vimeo"],
  ["instagram", "Instagram"],
  ["facebook", "Facebook"],
  ["reddit", "Reddit"],
  ["dailymotion", "Dailymotion"],
  ["bandcamp", "Bandcamp"],
  ["bilibili", "Bilibili"],
  ["niconico", "Niconico"],
];

export function siteName(key: string | null | undefined): string | null {
  if (!key) return null;
  const lower = key.toLowerCase();
  if (lower === "generic") return null;
  return SITE_NAMES.find(([prefix]) => lower.startsWith(prefix))?.[1] ?? key;
}

/** "video" or "audio file", for counts of downloads in a given mode. */
export function itemNoun(mode: string, count: number): string {
  const audio = mode === "audio";
  if (count === 1) return audio ? "audio file" : "video";
  return audio ? "audio files" : "videos";
}
