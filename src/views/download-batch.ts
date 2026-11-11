import { api, type ProbeResult } from "../api";
import type { EstimateRun } from "../download-estimates";
import { describeEnqueue, remainingItems, selectable, type BatchRow } from "../download-input";
import { folderName } from "../format";
import { capChoices } from "../resolution";
import { store, type BatchSession, type EstimateItem } from "../store";
import { button, chipRadio, segmented, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { group, row, sectionTitle } from "../ui/layout";
import {
  AUDIO_FORMATS,
  ROWS_PER_FRAME,
  audioHint,
  batchOptions,
  estimateButton,
  estimateLine,
  estimateSettings,
  paintRun,
  presetRow,
  qualityChips,
  renderIssues,
  rowNote,
  runKey,
  thumbnailEl,
  type BatchRowRefs,
} from "./download-shared";

export interface BatchDeps {
  run: EstimateRun<EstimateItem>;
  input: HTMLTextAreaElement;
  stage: HTMLElement;
  refreshCurrent(): void;
}

export interface BatchView {
  el: HTMLElement;
  paintRow(row: BatchRow<ProbeResult>): void;
  refresh(): void;
  dispose(): void;
  paintEstimate(): void;
}

export function mountBatch(session: BatchSession, deps: BatchDeps): BatchView {
  const { run: batchRun, input, stage } = deps;

  async function addBatch() {
    const session = store.batch;
    if (!session || store.links.submitting) return;
    const options = batchOptions(session.choices);
    const sending = store.links.enqueue(async (items) => {
      // No link-cache source: the cache records one looked-up link per call.
      const added = await api.enqueue(items, options, null);
      // Usually already applied from the jobs-added event; the revisions make this a no-op.
      store.addJobs(added);
      return added;
    });
    deps.refreshCurrent();
    const summary = await sending;
    deps.refreshCurrent();
    if (!summary) return;
    if (summary.added > 0 && store.batch === session) session.estimate = null;
    toast(describeEnqueue(summary), {
      tone: summary.added === 0 && summary.failed > 0 ? "error" : "info",
      action: summary.added > 0 ? { label: "View Queue", run: () => store.go("queue") } : undefined,
    });
    if (store.batch === session && store.links.rows.every((r) => r.added)) {
      const active = document.activeElement;
      const wasHere = !active || active === document.body || stage.contains(active);
      store.clearBatch();
      if (wasHere) input.focus({ preventScroll: true });
    }
  }

  const links = store.links;
  const rows = links.rows;
  const refs = new Map<number, BatchRowRefs>();
  let built = 0;
  let buildFrame = 0;
  let countFrame = 0;

  const counts = h("span", { class: "entries-count" });
  // The counts change many times a second while links are read, so a screen reader is
  // told only when reading starts again and when it ends, with the outcome.
  const announcer = h("span", { class: "visually-hidden", attrs: { role: "status", "aria-live": "polite" } });
  let wasBusy: boolean | null = null;
  // Stop and Retry hide themselves; focus moves to the one that appears instead.
  const swap = (action: () => void, next: () => HTMLElement) => () => {
    action();
    cancelAnimationFrame(countFrame);
    paintCounts();
    if (!next().hidden) next().focus();
  };
  const stopBtn: HTMLButtonElement = button({ label: "Stop", variant: "plain", size: "small", onClick: swap(() => links.cancel(), () => retryBtn) });
  stopBtn.title = "Stops reading the links not read yet. A lookup already running finishes in the background; its answer is ignored.";
  const retryBtn: HTMLButtonElement = button({ label: "Retry", icon: "retry", variant: "plain", size: "small", onClick: swap(() => links.retryAll(), () => stopBtn) });
  const allSelected = () => rows.some(selectable) && rows.every((r) => !selectable(r) || r.selected);
  // Select All and Download are marked unavailable (aria-disabled), never disabled: a
  // disabled button that holds focus drops the keyboard to the top of the page.
  const unavailable = (btn: HTMLElement) => btn.getAttribute("aria-disabled") === "true";
  const toggleAll = button({
    label: "Select All",
    variant: "plain",
    size: "small",
    onClick: () => {
      if (!unavailable(toggleAll)) links.selectAll(!allSelected());
    },
  });
  const clearBtn = button({
    label: "Clear",
    variant: "plain",
    size: "small",
    onClick: () => {
      store.clearBatch();
      input.focus();
    },
  });
  const selectedNote = h("span", { class: "save-to" });
  const batchLine = estimateLine(() => void batchRun.retryFailed(batchKey()), "Retry Failed", "Asks again only for the videos whose size could not be estimated. Sizes already read stay. Download is not affected.");
  // The items a download would send now: a row partly queued already counts only the rest.
  const batchItems = (): EstimateItem[] => {
    const options = batchOptions(session.choices);
    return rows
      .filter((r) => r.selected && selectable(r))
      .flatMap((r) => remainingItems(r))
      .map((item) => ({ url: item.url, playlistIndex: item.playlistIndex ?? null, options }));
  };
  const batchKey = () => runKey(batchOptions(session.choices), estimateSettings());
  const estimateBtn = estimateButton(batchRun, () => batchRun.start(batchItems(), batchKey()));
  const paintEstimate = () => {
    // Other choices, or an estimated link unselected or sent to the queue: the estimate
    // no longer applies (follow resets it, which repaints). A link that finishes reading
    // while a run goes is selected on its own; it does not end the run, it is named as
    // not included.
    if (!batchRun.follow(batchKey(), batchItems())) return;
    paintRun(batchRun, batchLine, estimateBtn, links.counts().selectedItems);
  };
  const addBtn = button({
    label: "Download",
    icon: "download",
    variant: "primary",
    onClick: () => {
      if (!unavailable(addBtn)) void addBatch();
    },
  });
  const addLabel = addBtn.querySelector("span")!;

  const paintCounts = () => {
    countFrame = 0;
    const c = links.counts();
    const ready = c.ready - c.added;
    counts.textContent = [
      ready > 0 ? `${ready} ready` : null,
      c.added > 0 ? `${c.added} added` : null,
      c.failed > 0 ? `${c.failed} failed` : null,
      c.looking > 0 ? `${c.looking} reading` : null,
      c.waiting > 0 ? `${c.waiting} waiting` : null,
      c.cancelled > 0 ? `${c.cancelled} stopped` : null,
    ]
      .filter(Boolean)
      .join("  ·  ");
    const busy = c.looking + c.waiting > 0;
    if (wasBusy !== null && busy !== wasBusy) {
      announcer.textContent = busy
        ? `Reading ${c.looking + c.waiting} ${c.looking + c.waiting === 1 ? "link" : "links"}.`
        : `Finished reading: ${counts.textContent}.`;
    }
    wasBusy = busy;
    // Stop or Retry may hide while it has focus (the reading it controls ended): focus
    // then moves to the other one, or to Select All, instead of falling to the page.
    const focused = document.activeElement;
    stopBtn.hidden = !busy;
    const again = c.failed + c.cancelled;
    retryBtn.hidden = again === 0;
    if ((focused === stopBtn || focused === retryBtn) && (focused as HTMLElement).hidden) {
      (focused === stopBtn && !retryBtn.hidden ? retryBtn : focused === retryBtn && !stopBtn.hidden ? stopBtn : toggleAll).focus();
    }
    retryBtn.querySelector("span")!.textContent = c.cancelled === 0 ? `Retry ${c.failed} Failed` : `Read ${again} Again`;
    toggleAll.querySelector("span")!.textContent = allSelected() ? "Select None" : "Select All";
    toggleAll.setAttribute("aria-disabled", String(!rows.some(selectable)));
    const n = c.selectedItems;
    addLabel.textContent = n === 0 ? "Download" : n === 1 ? "Download 1 Video" : `Download ${n} Videos`;
    addBtn.setAttribute("aria-disabled", String(n === 0 || links.submitting));
    selectedNote.textContent = `${c.selectedRows} of ${rows.length} links selected  ·  Saves to ${folderName(store.settings.outputDir)}`;
    paintEstimate();
  };
  const scheduleCounts = () => {
    if (!countFrame) countFrame = requestAnimationFrame(paintCounts);
  };

  const fallbackThumb = () => h("div", { class: "entry-thumb thumb-fallback" }, icon("link", 16));

  const paintRow = (r: BatchRow<ProbeResult>) => {
    scheduleCounts();
    const ref = refs.get(r.id);
    if (!ref) return;
    const can = selectable(r);
    ref.pick.setAttribute("aria-checked", String(r.selected));
    // aria-disabled, not disabled: the row keeps keyboard focus when it changes.
    ref.pick.setAttribute("aria-disabled", String(!can));
    ref.pick.dataset.status = r.added ? "added" : r.status;
    ref.title.textContent = r.result?.title || r.url;
    ref.title.title = r.url;
    const [note, tone] = rowNote(r);
    ref.sub.textContent = note;
    ref.sub.dataset.tone = tone;
    const art = r.result ? (r.result.thumbnail ?? r.result.entries.find((e) => e.thumbnail)?.thumbnail ?? null) : null;
    if (art !== ref.thumbFor) {
      const next = art ? thumbnailEl(art, "entry-thumb") : fallbackThumb();
      ref.thumb.replaceWith(next);
      ref.thumb = next;
      ref.thumbFor = art;
    }
    const side = r.status === "failed" || r.status === "cancelled" ? "retry" : "";
    if (side !== ref.sideFor) {
      ref.sideFor = side;
      replace(
        ref.side,
        side
          ? button({
              icon: "retry",
              title: "Read this link again",
              variant: "plain",
              size: "small",
              onClick: () => {
                links.retry(r.id);
                ref.pick.focus();
              },
            })
          : null,
      );
    }
  };

  const buildRow = (r: BatchRow<ProbeResult>): HTMLElement => {
    const thumb = fallbackThumb();
    const title = h("span", { class: "entry-title" });
    const sub = h("span", { class: "entry-sub" });
    const pick = h(
      "button",
      { class: "entry batch-pick", attrs: { type: "button", role: "checkbox", "aria-checked": "false" } },
      h("span", { class: "check" }, icon("check", 12)),
      thumb,
      h("span", { class: "entry-text" }, title, sub),
    );
    pick.addEventListener("click", () => links.setSelected(r.id, !r.selected));
    const side = h("span", { class: "batch-side" });
    refs.set(r.id, { pick, thumb, thumbFor: null, title, sub, side, sideFor: "" });
    paintRow(r);
    return h("div", { class: "batch-row" }, pick, side);
  };

  const list = h("div", { class: "batch-list", attrs: { role: "group", "aria-label": "Links in this batch" } });
  const buildSome = () => {
    buildFrame = 0;
    const end = Math.min(built + ROWS_PER_FRAME, rows.length);
    const fragment = document.createDocumentFragment();
    for (; built < end; built++) fragment.append(buildRow(rows[built]));
    list.append(fragment);
    if (built < rows.length) buildFrame = requestAnimationFrame(buildSome);
  };
  buildSome();

  // Choices shared by the whole batch. The rows are shown or hidden, never rebuilt, so
  // the control in use keeps focus.
  const choices = session.choices;
  const qualityRow = row(
    "Quality",
    qualityChips(capChoices(), choices.maxHeight, (value) => {
      choices.maxHeight = value;
      scheduleCounts();
    }),
    "Each video is saved at the largest size it has, up to this limit. Nothing is upscaled.",
  );
  const presetChoice = presetRow(choices.videoPreset, (next) => {
    choices.videoPreset = next;
    scheduleCounts();
  });
  const formatRow = row(
    "Format",
    chipRadio<string>("Audio format", AUDIO_FORMATS, choices.audioFormat, (value) => {
      choices.audioFormat = value;
      scheduleCounts();
    }),
    audioHint(),
  );
  const showMode = () => {
    qualityRow.hidden = choices.mode !== "video";
    presetChoice.hidden = choices.mode !== "video";
    formatRow.hidden = choices.mode !== "audio";
  };
  const typeSeg = segmented<"video" | "audio">(
    "Type",
    [
      { value: "video", label: "Video", icon: "video" },
      { value: "audio", label: "Audio Only", icon: "music" },
    ],
    choices.mode,
    (next) => {
      choices.mode = next;
      showMode();
      scheduleCounts();
    },
  );
  showMode();

  const view = {
    paintRow,
    refresh: scheduleCounts,
    dispose: () => {
      cancelAnimationFrame(buildFrame);
      cancelAnimationFrame(countFrame);
    },
  };
  paintCounts();

  const el = h(
    "div",
    { class: "result batch" },
    renderIssues(session.parsed),
    sectionTitle("Options"),
    group(
      row("Type", typeSeg.el, "Applies to every link here. Subtitles and transcripts can be chosen when a link is read on its own."),
      qualityRow,
      presetChoice,
      formatRow,
    ),
    h("div", { class: "entries-head" }, sectionTitle("Links"), counts, announcer, stopBtn, retryBtn, estimateBtn, toggleAll, clearBtn),
    h("div", { class: "glass group" }, list),
    batchLine.root,
    h("div", { class: "actions" }, selectedNote, addBtn),
  );
  return { el, ...view, paintEstimate };
}
