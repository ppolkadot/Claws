/** What a progress bar shows: the parts of a job that decide how its fill may move. */
export interface ProgressState {
  status: string;
  progress: number;
  progressKnown?: boolean;
}

/**
 * How a bar moves from what it showed to what it must show now. The fill may glide only
 * forwards within one continuing known-size download; everything else (the first paint,
 * a lower value, any status change such as completion, pause, failure or retry, a size
 * becoming known or unknown, a hidden window) snaps. So the bar never slides backwards,
 * never lags behind a finished download, and never animates what nobody can see.
 */
export function progressMotion(previous: ProgressState | undefined, next: ProgressState, hidden = false): "smooth" | "snap" {
  if (hidden || !previous) return "snap";
  if (previous.status !== next.status || Boolean(previous.progressKnown) !== Boolean(next.progressKnown)) return "snap";
  return next.progress >= previous.progress ? "smooth" : "snap";
}
