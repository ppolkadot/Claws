
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

export function sameTaskbarState(a: TaskbarState, b: TaskbarState): boolean {
  return a.status === b.status && (a.status !== "normal" || (b.status === "normal" && a.progress === b.progress));
}
