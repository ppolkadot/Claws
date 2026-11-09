import { api, errorMessage, type Job } from "../api";
import { folderName, formatBytes, formatEta, formatSpeed } from "../format";
import { keptKey, keptLabel } from "../kept-files";
import { store } from "../store";
import { button, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { fileDragButton } from "../ui/file-drag";
import { paintJobStatus } from "../ui/job-status";
import { progressBar, setProgress } from "../ui/layout";
import { canAnimate, cancelMotion, collapseOut, expandIn } from "../ui/motion";
import { progressMotion } from "../ui/progress-motion";
import { orderMoves } from "../ui/row-order";
import { videoOptionsText } from "../resolution";
import { transcriptText } from "../transcript-options";
import { showKept } from "./library";
import type { MountView } from "./shell";

const fail = (error: unknown) => toast(errorMessage(error), { tone: "error" });

interface RowRefs {
  el: HTMLElement;
  status: HTMLElement;
  bar: HTMLElement;
  actions: HTMLElement;
  thumb: HTMLElement;
  /** Title of the row's button that had keyboard focus when it was pressed, until the repaint it caused. */
  refocus: string | null;
  /** A button was pressed and the repaint it causes has not come yet. */
  pressed: boolean;
  /** Until this time (performance.now()), the row's buttons ignore activation. */
  guardUntil: number;
  /** Counts the row's presses, so an older press's fallback timer leaves a newer press's state alone. */
  pressId: number;
  /** What the row last showed about kept subtitle files (kept-files.ts keptKey). */
  kept: string;
}

/** A percentage only when every stream's size is known; "about" when a size is an estimate. */
function percent(job: Job): string | null {
  if (!job.progressKnown) return null;
  const pct = `${Math.round(job.progress * 100)}%`;
  return job.progressEstimated ? `About ${pct}` : pct;
}

/** The job's Library entry, which holds what a finished download kept (the Queue forgets it). */
function libraryEntry(job: Job) {
  return job.status === "completed" ? store.history.find((h) => h.id === job.id) : undefined;
}

function statusText(job: Job): string {
  // A transcript fetches captions only: no bytes to count, so no percentage either.
  if (job.options.mode === "transcript" && (job.status === "downloading" || job.status === "processing")) return "Getting captions…";
  switch (job.status) {
    case "queued":
      return "Waiting for a free slot";
    case "downloading": {
      const amount = percent(job) ?? (job.downloadedBytes ? `${formatBytes(job.downloadedBytes)} downloaded` : "Starting");
      // Time left only when the total and a real speed support it (the backend decides).
      const left = job.eta != null ? (job.progressEstimated ? `about ${formatEta(job.eta)}` : formatEta(job.eta)) : null;
      return [amount, formatSpeed(job.speed), left].filter(Boolean).join("  ·  ");
    }
    case "processing":
      return "Processing the file";
    case "paused": {
      const at = percent(job) ?? (job.downloadedBytes ? formatBytes(job.downloadedBytes) : job.progress > 0 ? `${Math.round(job.progress * 100)}%` : null);
      return at ? `Paused at ${at}` : "Paused";
    }
    case "completed": {
      const saved = job.filePath ? `Saved to ${folderName(job.filePath.replace(/[\\/][^\\/]*$/, ""))}` : "Completed";
      return [saved, keptLabel(libraryEntry(job))].filter(Boolean).join("  ·  ");
    }
    case "cancelled":
      return "Cancelled";
    case "failed":
      return job.error || "Download failed";
  }
}

function optionsText(job: Job): string {
  const o = job.options;
  if (o.mode === "transcript") return transcriptText(o);
  if (o.mode === "audio") return o.audioFormat === "best" ? "Audio" : o.audioFormat.toUpperCase();
  return videoOptionsText(o.maxHeight, o.videoPreset);
}

/** The symbol shown in place of a missing thumbnail. */
function modeIcon(mode: Job["options"]["mode"]) {
  return mode === "audio" ? "music" : mode === "transcript" ? "captions" : "video";
}

/** New rows built per frame during a large addition: about a frame's worth of work. */
const ROWS_PER_FRAME = 40;
/** At most this many rows arrive or leave with movement at once; more (a playlist, Clear Finished) simply appear or go. */
const ANIMATED_ROWS = 12;
/** Buttons focus never jumps to on its own: a stray Enter must not cancel or remove. */
const DESTRUCTIVE = new Set(["Cancel download", "Remove from list"]);
/**
 * After a press repaints a row's buttons (Pause becomes Resume, Cancel becomes Try again)
 * or removes the row, the buttons now under the pointer or holding focus ignore activation
 * this long, so a double click or a quick second Enter cannot undo the first.
 */
const PRESS_GUARD_MS = 600;

/** Arms a row's guard: its buttons ignore activation for PRESS_GUARD_MS. */
function guard(refs: RowRefs) {
  refs.guardUntil = performance.now() + PRESS_GUARD_MS;
}

/** Where focus may land by itself: the same action if offered and not destructive, else the first that is not. */
function focusable(buttons: HTMLElement[], title: string | null): HTMLElement | undefined {
  return buttons.find((b) => b.title === title && !DESTRUCTIVE.has(b.title)) ?? buttons.find((b) => !DESTRUCTIVE.has(b.title));
}

function buildRow(job: Job): RowRefs {
  const status = h("span", { class: "job-status" });
  const bar = progressBar(0);
  const actions = h("div", { class: "job-actions" });
  // A held Enter repeats keydown, and every repeat would click whatever button has focus by then.
  actions.addEventListener("keydown", (event) => {
    if (event.repeat && event.key === "Enter") event.preventDefault();
  });
  const thumb = job.thumbnail
    ? h("img", { class: "job-thumb", attrs: { src: job.thumbnail, alt: "", decoding: "async", referrerpolicy: "no-referrer", draggable: false } })
    : h("div", { class: "job-thumb thumb-fallback" }, icon(modeIcon(job.options.mode), 18));
  thumb.addEventListener("error", () => thumb.replaceWith(h("div", { class: "job-thumb thumb-fallback" }, icon(modeIcon(job.options.mode), 18))));
  const el = h(
    "li",
    { class: "job" },
    thumb,
    h(
      "div",
      { class: "job-main" },
      h("div", { class: "job-head" }, h("span", { class: "job-title", text: job.title, attrs: { title: job.title } }), h("span", { class: "job-kind", text: optionsText(job) })),
      status,
      bar,
    ),
    actions,
  );
  return { el, status, bar, actions, thumb, refocus: null, pressed: false, guardUntil: 0, pressId: 0, kept: "" };
}

function paintActions(refs: RowRefs, job: Job) {
  // Keyboard focus survives the buttons being replaced by the new status's: the same action
  // when it is still offered, otherwise the first one that cannot cancel or remove.
  const active = document.activeElement;
  const nowhere = !active || active === document.body;
  const focused = active instanceof HTMLElement && refs.actions.contains(active) ? active.title : nowhere ? refs.refocus : null;
  refs.refocus = null;
  // This repaint answers a press: the new buttons (now under the pointer or the focus) wait
  // out the guard, so a second click or Enter cannot resume what was just paused or retry
  // what was just cancelled.
  if (refs.pressed) guard(refs);
  refs.pressed = false;
  const items: HTMLElement[] = [];
  // Disabled until the new status repaints them, so a double click cannot act on a job
  // that is already pausing or cancelling. Re-enabled if nothing changes.
  const act = (run: () => Promise<unknown>) => () => {
    if (performance.now() < refs.guardUntil) return;
    const press = ++refs.pressId;
    const pressed = document.activeElement;
    refs.refocus = pressed instanceof HTMLElement && refs.actions.contains(pressed) ? pressed.title : null;
    refs.pressed = true;
    const buttons = [...refs.actions.querySelectorAll<HTMLButtonElement>("button")];
    buttons.forEach((b) => (b.disabled = true));
    void run()
      .catch(fail)
      .finally(() =>
        window.setTimeout(() => {
          buttons.forEach((b) => (b.disabled = false));
          // Only this press's own state: a newer press on the row (its status change may
          // still be on the way, as when a running download is paused) keeps its focus
          // target and its guard.
          if (refs.pressId !== press) return;
          refs.refocus = null;
          refs.pressed = false;
        }, 1500),
      );
  };
  switch (job.status) {
    case "downloading":
      items.push(button({ icon: "pause", title: "Pause", variant: "plain", onClick: act(() => api.pause(job.id)) }));
      items.push(button({ icon: "x", title: "Cancel download", variant: "plain", onClick: act(() => api.cancel(job.id)) }));
      break;
    case "queued":
      items.push(button({ icon: "pause", title: "Pause", variant: "plain", onClick: act(() => api.pause(job.id)) }));
      items.push(button({ icon: "x", title: "Cancel download", variant: "plain", onClick: act(() => api.cancel(job.id)) }));
      break;
    case "processing":
      items.push(button({ icon: "x", title: "Cancel download", variant: "plain", onClick: act(() => api.cancel(job.id)) }));
      break;
    case "paused":
      items.push(button({ icon: "play", title: "Resume", variant: "plain", onClick: act(() => api.resume(job.id)) }));
      items.push(button({ icon: "x", title: "Cancel download", variant: "plain", onClick: act(() => api.cancel(job.id)) }));
      break;
    case "failed":
    case "cancelled":
      items.push(button({ icon: "retry", title: "Try again", variant: "plain", onClick: act(() => api.resume(job.id)) }));
      items.push(button({ icon: "x", title: "Remove from list", variant: "plain", onClick: act(() => api.remove(job.id)) }));
      break;
    case "completed":
      items.push(button({ icon: "folder", title: "Show in folder", variant: "plain", onClick: act(() => api.revealDownload(job.id)) }));
      // Only while the Library entry records kept files: they outlive this row.
      if (keptLabel(libraryEntry(job))) items.push(button({ icon: "captions", title: "Show kept files", variant: "plain", onClick: act(() => showKept(job.id)) }));
      if (job.filePath) items.push(fileDragButton(job.id));
      items.push(button({ icon: "x", title: "Remove from list", variant: "plain", onClick: act(() => api.remove(job.id)) }));
      break;
  }
  replace(refs.actions, ...items);
  if (focused === null) return;
  const next = items.find((b) => b.title === focused) ?? items.find((b) => !DESTRUCTIVE.has(b.title));
  next?.focus({ preventScroll: true });
}

function paintRow(refs: RowRefs, job: Job, previous: Job | undefined) {
  refs.el.dataset.status = job.status;
  const changed = !!previous && previous.status !== job.status;
  const kept = keptKey(libraryEntry(job));
  const keptChanged = refs.kept !== kept;
  refs.kept = kept;
  paintJobStatus(refs.status, statusText(job), job.status === "completed", changed, () => icon("check", 12));
  refs.status.title = job.status === "failed" ? job.error ?? "" : "";
  const unknown = job.status === "processing" || (job.status === "downloading" && !job.progressKnown);
  // The fill glides only forwards within one download; completion, retry and the rest snap.
  setProgress(refs.bar, unknown ? null : job.status === "completed" ? 1 : job.progress, progressMotion(previous, job, document.hidden));
  if (!previous || previous.status !== job.status || keptChanged) paintActions(refs, job);
}

export const mountQueue: MountView = (container, ctx) => {
  const clearBtn = button({
    label: "Clear Finished",
    variant: "secondary",
    size: "small",
    onClick: () => void api.clearFinished().catch(fail),
  });
  ctx.setActions([clearBtn]);

  const list = h("ul", { class: "jobs glass", attrs: { "aria-label": "Downloads" } });
  const empty = h(
    "div",
    { class: "empty" },
    h("div", { class: "empty-symbol" }, icon("list", 46)),
    h("h2", { class: "empty-title", text: "Nothing is downloading" }),
    h("p", { class: "empty-text", text: "Links you download appear here with live progress. Finished files go to your Library." }),
    button({ label: "Paste a Link", icon: "link", variant: "primary", onClick: () => store.go("download") }),
  );
  container.append(list, empty);

  const rows = new Map<string, RowRefs>();
  const known = new Map<string, Job>();
  // Each row element's refs, for finding a neighbour when focus has to move.
  const refsOf = new Map<Element, RowRefs>();
  // Removed rows still collapsing out: no longer in `rows`, but leaving the view stops them.
  const collapsing = new Set<HTMLElement>();
  const emptyAction = empty.querySelector<HTMLElement>("button");

  /**
   * Where keyboard focus goes when the row holding it is removed. Called before the rows
   * go; the returned step runs once the list (or the empty state) is in place. The
   * neighbour's buttons are guarded, so the Enter that removed one row cannot also act on
   * the next.
   */
  const focusAfterRemoval = (going: RowRefs[]): (() => void) | null => {
    const active = document.activeElement;
    // A pressed button is disabled at once and loses focus to <body>; its row remembers it.
    const nowhere = !active || active === document.body;
    const from = going.find((refs) => (active ? refs.el.contains(active) : false)) ?? (nowhere ? going.find((refs) => refs.refocus !== null) : undefined);
    if (!from) return null;
    const title = from.refocus ?? (active instanceof HTMLElement ? active.title : null);
    const leaving = new Set<Element>(going.map((refs) => refs.el));
    const children = [...list.children];
    const at = children.indexOf(from.el);
    const staying = (el: Element) => !leaving.has(el) && refsOf.has(el);
    const neighbour = children.slice(at + 1).find(staying) ?? children.slice(0, at).reverse().find(staying);
    const refs = neighbour ? refsOf.get(neighbour) : undefined;
    return () => {
      if (refs) {
        guard(refs);
        const target = focusable([...refs.actions.querySelectorAll<HTMLElement>("button")], title);
        if (target) return target.focus({ preventScroll: true });
        // Only destructive buttons there (a job being processed): the list itself, never "Cancel".
        list.tabIndex = -1;
        return list.focus({ preventScroll: true });
      }
      if (!empty.hidden) emptyAction?.focus({ preventScroll: true });
    };
  };
  // True while a large addition (or the rows already there when the page opens) is still
  // being built: none of its rows animate, including the last frame's few.
  let filling = true;
  let frame = 0;
  let leaveTimer = 0;
  // Dev builds expose counters for scripts/dev/queue-bench.mjs.
  const stats = { structuralSyncs: 0, rowPaints: 0 };
  if (import.meta.env.DEV) (window as unknown as { __clawsQueueStats: typeof stats }).__clawsQueueStats = stats;

  const paint = (refs: RowRefs, job: Job) => {
    stats.rowPaints += 1;
    paintRow(refs, job, known.get(job.id));
    known.set(job.id, job);
  };

  const paintClear = () => {
    clearBtn.disabled = store.finishedCount() === 0;
  };

  // Adding or removing jobs, at most once per frame. A large addition is built
  // ROWS_PER_FRAME rows per frame so no single frame blocks the window. Rows that animate
  // are measured together (one layout read before the changes, one after), never per row.
  const sync = () => {
    frame = 0;
    stats.structuralSyncs += 1;
    const jobs = store.jobList();
    const ids = new Set(jobs.map((j) => j.id));
    const gone = [...rows].filter(([id]) => !ids.has(id));
    // Focus inside a row that is going moves to the neighbouring row (the next, else the
    // previous), or to the empty state's action once the list is empty.
    const refocus = focusAfterRemoval(gone.map(([, refs]) => refs));
    // Clear Finished on a long list would be dozens of height animations: those just go.
    const animateOut = gone.length <= ANIMATED_ROWS && canAnimate();
    const heightsOut = animateOut ? gone.map(([, refs]) => refs.el.getBoundingClientRect().height) : [];
    gone.forEach(([id, refs], i) => {
      cancelMotion(refs.status);
      if (animateOut) {
        // A row on its way out can no longer be focused or pressed, nor is it read out.
        refs.el.inert = true;
        refs.el.setAttribute("aria-hidden", "true");
        const collapse = collapseOut(refs.el, heightsOut[i]);
        if (collapse) {
          collapsing.add(refs.el);
          collapse.addEventListener("finish", () => collapsing.delete(refs.el));
        }
      } else {
        cancelMotion(refs.el);
        refs.el.remove();
      }
      rows.delete(id);
      refsOf.delete(refs.el);
      known.delete(id);
    });
    const fresh = jobs.filter((job) => !rows.has(job.id));
    // A playlist's worth of rows at once would be hundreds of springs: only small additions animate.
    const animateIn = !filling && fresh.length <= ANIMATED_ROWS && canAnimate();
    const batch = fresh.slice(0, ROWS_PER_FRAME);
    const entering: HTMLElement[] = [];
    for (const job of batch) {
      const refs = buildRow(job);
      rows.set(job.id, refs);
      refsOf.set(refs.el, refs);
      paint(refs, job);
      list.appendChild(refs.el);
      if (animateIn) entering.push(refs.el);
    }
    filling = batch.length < fresh.length;
    if (filling) scheduleSync();
    // Jobs keep their creation order, so this normally moves nothing. Rows collapsing out
    // are skipped and stay where they were.
    const desired = jobs.flatMap((job) => rows.get(job.id)?.el ?? []);
    const placed = new Set<Element>(desired);
    const children = [...list.children];
    const leavingRows = new Set(children.filter((el) => !placed.has(el)));
    for (const { key, before } of orderMoves<Element>(children, desired, leavingRows)) list.insertBefore(key, before);
    // Rows collapsing out still occupy the list for a moment.
    const leaving = list.children.length - jobs.length;
    list.hidden = jobs.length === 0 && leaving <= 0;
    empty.hidden = jobs.length > 0;
    window.clearTimeout(leaveTimer);
    if (jobs.length === 0 && leaving > 0) leaveTimer = window.setTimeout(scheduleSync, 360);
    paintClear();
    refocus?.();
    // Measured once the list is shown: a first row into an empty Queue has its real height.
    const heightsIn = entering.map((el) => el.getBoundingClientRect().height);
    entering.forEach((el, i) => expandIn(el, heightsIn[i]));
  };

  const scheduleSync = () => {
    if (!frame) frame = requestAnimationFrame(sync);
  };

  // One job changed: repaint its row only. A job without a row yet is drawn by the
  // pending structural sync.
  const offJob = store.on("job", (detail) => {
    const job = detail as Job;
    const refs = rows.get(job.id);
    if (refs) paint(refs, job);
  });
  const offJobs = store.on("jobs", scheduleSync);
  // The Library entry (with its kept files) can arrive after the job's completion.
  const offHistory = store.on("history", () => {
    for (const [id, refs] of rows) {
      const job = known.get(id);
      if (job?.status === "completed" && refs.kept !== keptKey(libraryEntry(job))) paint(refs, job);
    }
  });
  const offStatus = store.on("jobStatus", paintClear);
  sync();

  return () => {
    offJob();
    offJobs();
    offHistory();
    offStatus();
    cancelAnimationFrame(frame);
    window.clearTimeout(leaveTimer);
    for (const refs of rows.values()) cancelMotion(refs.el, refs.status);
    // Rows still collapsing out are no longer in `rows`: stop them and take them out now.
    for (const el of collapsing) {
      cancelMotion(el);
      el.remove();
    }
    collapsing.clear();
  };
};
