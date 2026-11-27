
export interface SummaryJob {
  status: string;
  speed: number | null;
  eta: number | null;
  progressKnown?: boolean;
  progressEstimated?: boolean;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
}

export interface QueueSummary {
  downloading: number;
  processing: number;
  queued: number;
  paused: number;
  failed: number;
  speed: number | null;
  /** Seconds until every transfer is done; null unless that can be said (see `summarize`). */
  eta: number | null;
  /** The time left rests on an estimated size. */
  etaApproximate: boolean;
}

/**
 * Counts by status, the combined speed and the time left. The time left is the longest of
 * the running transfers' own, and is given only when every running transfer reported one and
 * nothing is still waiting in line: a waiting download has no size or speed to go by yet.
 */
export function summarize(jobs: Iterable<SummaryJob>): QueueSummary {
  const summary: QueueSummary = { downloading: 0, processing: 0, queued: 0, paused: 0, failed: 0, speed: null, eta: null, etaApproximate: false };
  let etaKnown = true;
  let longest = 0;
  for (const job of jobs) {
    switch (job.status) {
      case "downloading":
        summary.downloading += 1;
        if (job.speed != null && job.speed > 0) summary.speed = (summary.speed ?? 0) + job.speed;
        if (job.eta == null) etaKnown = false;
        else {
          longest = Math.max(longest, job.eta);
          summary.etaApproximate ||= job.progressEstimated === true;
        }
        break;
      case "processing":
        summary.processing += 1;
        break;
      case "queued":
        summary.queued += 1;
        break;
      case "paused":
        summary.paused += 1;
        break;
      case "failed":
        summary.failed += 1;
        break;
    }
  }
  if (summary.downloading > 0 && etaKnown && summary.queued === 0) summary.eta = longest;
  else summary.etaApproximate = false;
  return summary;
}

export type TaskbarState = { status: "none" } | { status: "indeterminate" } | { status: "normal"; progress: number };

/**
 * The taskbar button's progress: the bytes received out of the bytes expected across the
 * downloads that are running. Nothing running shows no bar; a running download whose size
 * is unknown makes the bar indeterminate, not a made-up fraction. A download being
 * processed has all its bytes.
 */
export function taskbarState(jobs: Iterable<SummaryJob>): TaskbarState {
  let received = 0;
  let expected = 0;
  let running = 0;
  for (const job of jobs) {
    if (job.status !== "downloading" && job.status !== "processing") continue;
    running += 1;
    const total = job.totalBytes;
    if (!job.progressKnown || total == null || total <= 0) return { status: "indeterminate" };
    expected += total;
    received += job.status === "processing" ? total : Math.min(job.downloadedBytes ?? 0, total);
  }
  if (running === 0) return { status: "none" };
  return { status: "normal", progress: Math.round((received / expected) * 100) };
}

/** A running Wallpaper Engine export, as the taskbar sees it. */
export interface ExportActivity {
  running: boolean;
  /** How far along, 0 to 1; null when the stage counts nothing (the sheen). */
  fraction: number | null;
}

/**
 * The taskbar button with a running export added. With nothing else running, the export's own
 * fraction shows (or the indeterminate bar while it counts nothing). Beside running downloads
 * the two are averaged when both are counted, and the bar is indeterminate when either is not.
 */
export function withExport(downloads: TaskbarState, activity: ExportActivity | null): TaskbarState {
  if (!activity?.running) return downloads;
  const own: TaskbarState = activity.fraction === null ? { status: "indeterminate" } : { status: "normal", progress: Math.round(Math.min(1, Math.max(0, activity.fraction)) * 100) };
  if (downloads.status === "none") return own;
  if (downloads.status === "indeterminate" || own.status === "indeterminate") return { status: "indeterminate" };
  return { status: "normal", progress: Math.round((downloads.progress + own.progress) / 2) };
}

export function sameTaskbarState(a: TaskbarState, b: TaskbarState): boolean {
  return a.status === b.status && (a.status !== "normal" || (b.status === "normal" && a.progress === b.progress));
}

/**
 * The Queue's summary line, at most three facts: while something transfers, how many and how
 * fast (with the time left when it is known), plus failures, which need action. With nothing
 * transferring, only what needs the user: paused and failed. The rows show the rest.
 */
export function summaryParts(s: QueueSummary, speed: (bytesPerSecond: number) => string, eta: (seconds: number) => string): string[] {
  if (s.downloading > 0) {
    const pace = [s.speed !== null ? speed(s.speed) : "", s.eta !== null ? `${s.etaApproximate ? "about " : ""}${eta(s.eta)}` : ""].filter(Boolean).join(", ");
    return [`${s.downloading} downloading`, pace, s.failed > 0 ? `${s.failed} failed` : ""].filter(Boolean);
  }
  return [s.paused > 0 ? `${s.paused} paused` : "", s.failed > 0 ? `${s.failed} failed` : ""].filter(Boolean);
}
