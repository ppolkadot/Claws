import { api, errorMessage, type Job } from "../api";
import { folderName, formatEta, formatSpeed } from "../format";
import { store } from "../store";
import { button, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { progressBar, setProgress } from "../ui/layout";
import { collapseOut, expandIn } from "../ui/motion";
import type { MountView } from "./shell";

const fail = (error: unknown) => toast(errorMessage(error), { tone: "error" });

interface RowRefs {
  el: HTMLElement;
  status: HTMLElement;
  bar: HTMLElement;
  actions: HTMLElement;
  thumb: HTMLElement;
}

function statusText(job: Job): string {
  const pct = `${Math.round(job.progress * 100)}%`;
  switch (job.status) {
    case "queued":
      return "Waiting for a free slot";
    case "downloading":
      return [pct, formatSpeed(job.speed), formatEta(job.eta)].filter(Boolean).join("  ·  ");
    case "processing":
      return "Processing the file";
    case "paused":
      return job.progress > 0 ? `Paused at ${pct}` : "Paused";
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
  if (o.mode === "audio") return o.audioFormat === "best" ? "Audio" : o.audioFormat.toUpperCase();
  return o.maxHeight ? `Video up to ${o.maxHeight}p` : "Video, best quality";
}

function buildRow(job: Job): RowRefs {
  const status = h("span", { class: "job-status" });
  const bar = progressBar(0);
  const actions = h("div", { class: "job-actions" });
  const thumb = job.thumbnail
    ? h("img", { class: "job-thumb", attrs: { src: job.thumbnail, alt: "", decoding: "async", referrerpolicy: "no-referrer", draggable: false } })
    : h("div", { class: "job-thumb thumb-fallback" }, icon(job.options.mode === "audio" ? "music" : "video", 18));
  thumb.addEventListener("error", () => thumb.replaceWith(h("div", { class: "job-thumb thumb-fallback" }, icon("video", 18))));
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
  const act = (run: () => Promise<unknown>) => () => void run().catch(fail);
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
      items.push(button({ icon: "x", title: "Remove from list", variant: "plain", onClick: act(() => api.remove(job.id)) }));
      break;
  }
  replace(refs.actions, ...items);
}

function paintRow(refs: RowRefs, job: Job, previous: Job | undefined) {
  refs.el.dataset.status = job.status;
  refs.status.textContent = statusText(job);
  refs.status.title = job.status === "failed" ? job.error ?? "" : "";
  setProgress(refs.bar, job.status === "processing" ? null : job.status === "completed" ? 1 : job.progress);
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

  const sync = () => {
    const jobs = store.jobList();
    const ids = new Set(jobs.map((j) => j.id));
    for (const [id, refs] of rows) {
      if (!ids.has(id)) {
        collapseOut(refs.el);
        rows.delete(id);
        known.delete(id);
      }
    }
    for (const job of jobs) {
      let refs = rows.get(job.id);
      if (!refs) {
        refs = buildRow(job);
        rows.set(job.id, refs);
        list.appendChild(refs.el);
        if (mounted) expandIn(refs.el);
      }
      paintRow(refs, job, known.get(job.id));
      known.set(job.id, job);
    }
    // Keep list order stable by creation time, moving only rows that are out of place
    // so a row mid-animation is never re-inserted.
    jobs.forEach((job, index) => {
      const el = rows.get(job.id)!.el;
      const at = list.children[index];
      if (at !== el) list.insertBefore(el, at ?? null);
    });
    // Rows collapsing out still occupy the list for a moment.
    const leaving = list.children.length - jobs.length;
    list.hidden = jobs.length === 0 && leaving <= 0;
    empty.hidden = jobs.length > 0;
    if (jobs.length === 0 && leaving > 0) window.setTimeout(sync, 360);
    clearBtn.disabled = !jobs.some((j) => j.status === "completed" || j.status === "cancelled");
  };

  const offJob = store.on("job", (detail) => {
    const job = detail as Job;
    const refs = rows.get(job.id);
    if (!refs) return sync();
    paintRow(refs, job, known.get(job.id));
    known.set(job.id, job);
    clearBtn.disabled = !store.jobList().some((j) => j.status === "completed" || j.status === "cancelled");
  });
  const offJobs = store.on("jobs", sync);
  sync();
  mounted = true;

  return () => {
    offJob();
    offJobs();
  };
};
