import { api, errorMessage, type Job } from "../api";
import { folderName, formatBytes, formatEta, formatSpeed } from "../format";
import { store } from "../store";
import { button, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { fileDragButton } from "../ui/file-drag";
import { completionFeedback } from "../ui/motion";
import { progressBar, setProgress } from "../ui/layout";
import { collapseOut, expandIn } from "../ui/motion";
import { videoOptionsText } from "../resolution";
import { transcriptText } from "../transcript-options";
import type { MountView } from "./shell";

const fail = (error: unknown) => toast(errorMessage(error), { tone: "error" });

interface RowRefs {
  el: HTMLElement;
  status: HTMLElement;
  bar: HTMLElement;
  actions: HTMLElement;
  thumb: HTMLElement;
}

/** A percentage only when every stream's size is known; "about" when a size is an estimate. */
function percent(job: Job): string | null {
  if (!job.progressKnown) return null;
  const pct = `${Math.round(job.progress * 100)}%`;
  return job.progressEstimated ? `About ${pct}` : pct;
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
    case "completed":
      return job.filePath ? `Saved to ${folderName(job.filePath.replace(/[\\/][^\\/]*$/, ""))}` : "Completed";
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

function buildRow(job: Job): RowRefs {
  const status = h("span", { class: "job-status" });
  const bar = progressBar(0);
  const actions = h("div", { class: "job-actions" });
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
  return { el, status, bar, actions, thumb };
}

function paintActions(refs: RowRefs, job: Job) {
  const items: Node[] = [];
  // Disabled until the new status repaints them, so a double click cannot act on a job
  // that is already pausing or cancelling. Re-enabled if nothing changes.
  const act = (run: () => Promise<unknown>) => () => {
    const buttons = [...refs.actions.querySelectorAll<HTMLButtonElement>("button")];
    buttons.forEach((b) => (b.disabled = true));
    void run()
      .catch(fail)
      .finally(() => window.setTimeout(() => buttons.forEach((b) => (b.disabled = false)), 1500));
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
      if (job.filePath) items.push(fileDragButton(job.id));
      items.push(button({ icon: "x", title: "Remove from list", variant: "plain", onClick: act(() => api.remove(job.id)) }));
      break;
  }
  replace(refs.actions, ...items);
}

function paintRow(refs: RowRefs, job: Job, previous: Job | undefined) {
  refs.el.dataset.status = job.status;
  refs.status.textContent = statusText(job);
  if (job.status === "completed") refs.status.prepend(icon("check", 12), " ");
  if (previous && previous.status !== job.status) completionFeedback(refs.status, job.status === "completed");
  refs.status.title = job.status === "failed" ? job.error ?? "" : "";
  const unknown = job.status === "processing" || (job.status === "downloading" && !job.progressKnown);
  setProgress(refs.bar, unknown ? null : job.status === "completed" ? 1 : job.progress);
  if (!previous || previous.status !== job.status) paintActions(refs, job);
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
  let mounted = false; // rows already there when the page opens appear without animating
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

  // Adding or removing jobs, at most once per frame. Only rows that are out of place move,
  // so a row mid-animation is never re-inserted. A large addition is built ROWS_PER_FRAME
  // rows per frame so no single frame blocks the window.
  const sync = () => {
    frame = 0;
    stats.structuralSyncs += 1;
    const jobs = store.jobList();
    const ids = new Set(jobs.map((j) => j.id));
    for (const [id, refs] of rows) {
      if (!ids.has(id)) {
        collapseOut(refs.el);
        rows.delete(id);
        known.delete(id);
      }
    }
    const fresh = jobs.filter((job) => !rows.has(job.id));
    // A playlist's worth of rows at once would be hundreds of springs: only small additions animate.
    const animate = mounted && fresh.length <= 12;
    const batch = fresh.slice(0, ROWS_PER_FRAME);
    for (const job of batch) {
      const refs = buildRow(job);
      rows.set(job.id, refs);
      paint(refs, job);
      list.appendChild(refs.el);
      if (animate) expandIn(refs.el);
    }
    if (batch.length < fresh.length) scheduleSync();
    jobs
      .filter((job) => rows.has(job.id))
      .forEach((job, index) => {
        const el = rows.get(job.id)!.el;
        const at = list.children[index];
        if (at !== el) list.insertBefore(el, at ?? null);
      });
    // Rows collapsing out still occupy the list for a moment.
    const leaving = list.children.length - jobs.length;
    list.hidden = jobs.length === 0 && leaving <= 0;
    empty.hidden = jobs.length > 0;
    window.clearTimeout(leaveTimer);
    if (jobs.length === 0 && leaving > 0) leaveTimer = window.setTimeout(scheduleSync, 360);
    paintClear();
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
  const offStatus = store.on("jobStatus", paintClear);
  sync();
  mounted = true;

  return () => {
    offJob();
    offJobs();
    offStatus();
    cancelAnimationFrame(frame);
    window.clearTimeout(leaveTimer);
  };
};
