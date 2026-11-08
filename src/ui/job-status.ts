import { completionFeedback } from "./motion.js";

/**
 * Writes a queue row's status line. The text replaces the whole content, so the static
 * check a completed row shows is never added twice however often the row repaints. The
 * short completion movement runs only when the status itself changed (never per progress
 * event), and a change away from completed cancels it.
 */
export function paintJobStatus(el: HTMLElement, text: string, completed: boolean, statusChanged: boolean, check: () => Node) {
  el.textContent = text;
  if (completed) el.prepend(check(), " ");
  if (statusChanged) completionFeedback(el, completed);
}
