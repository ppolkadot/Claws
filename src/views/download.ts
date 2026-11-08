import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type DownloadOptions, type ProbeResult } from "../api";
import { CallLimit, EstimateRun, PreviewEstimator, aggregate, describeRun, describeSingle } from "../download-estimates";
import {
  MAX_LINKS,
  describeEnqueue,
  isBatchInput,
  itemKey,
  parseLinks,
  remainingItems,
  selectable,
  type BatchRow,
  type ParsedInput,
} from "../download-input";
import { formatDate, formatDuration, folderName } from "../format";
import { store, type BatchChoices, type BatchSession } from "../store";
import { button, chipCheck, chipRadio, segmented, toast, toggle } from "../ui/components";
import { clear, h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { group, progressBar, row, sectionTitle, setProgress, spinner } from "../ui/layout";
import { settleIn } from "../ui/motion";
import type { MountView } from "./shell";

type Phase = "idle" | "loading" | "result" | "error" | "batch";

/** Batch rows built per frame, as in the Queue view: about a frame's worth of work. */
const ROWS_PER_FRAME = 40;
/** Skipped lines listed by name; the rest are counted. */
const ISSUES_SHOWN = 6;

const STANDARD_HEIGHTS = [2160, 1440, 1080, 720, 480, 360, 240, 144];
const AUDIO_FORMATS = [
  { value: "mp3", label: "MP3" },
  { value: "m4a", label: "M4A" },
  { value: "opus", label: "Opus" },
  { value: "flac", label: "FLAC" },
  { value: "wav", label: "WAV" },
  { value: "best", label: "Original" },
];

function qualityChoices(heights: number[], playlist: boolean): { value: number; label: string }[] {
  const label = (value: number) => (value === 2160 ? "4K" : `${value}p`);
  // A flat playlist read has no formats; the choice is a cap, so any height works.
  if (playlist && heights.length === 0) {
    return STANDARD_HEIGHTS.filter((h) => h >= 360).map((value) => ({ value, label: label(value) }));
  }
  const max = heights[0] ?? 0;
  let picks = STANDARD_HEIGHTS.filter((h) => heights.some((available) => available >= h * 0.95) && h <= max * 1.05);
  if (picks.length === 0) picks = heights.slice(0, 6);
  return picks.slice(0, 7).map((value) => ({ value, label: label(value) }));
}

export function looksLikeUrl(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
}

/** Text with two or more links on separate lines: filled in, never looked up by itself. */
export function looksLikeLinkList(text: string): boolean {
  return /[\r\n]/.test(text.trim()) && parseLinks(text).links.length >= 2;
}

function audioHint(): string {
  return store.settings.audioSelection === "preferMatching"
    ? "Original keeps the site's own audio. For other formats, a stream already in that format is saved without conversion when the site has one."
    : "Original keeps the site's own audio without re-encoding. Other formats are converted from the site's best audio.";
}

const shorten = (text: string, max = 70) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

function batchOptions(choices: BatchChoices): DownloadOptions {
  return {
    mode: choices.mode,
    maxHeight: choices.mode === "video" ? choices.maxHeight : null,
    audioFormat: choices.audioFormat,
    subtitleLangs: [],
    autoSubs: false,
    embedSubs: false,
    fastSave: false,
  };
}

/** One item to estimate, with the choices it was asked for. */
interface EstimateItem {
  url: string;
  playlistIndex: number | null;
  options: DownloadOptions;
}

type EstimateLine = {
  root: HTMLElement;
  text: HTMLElement;
  size: HTMLElement;
  detail: HTMLElement;
  /** Visually hidden; written only when an estimate starts or settles, never per step. */
  status: HTMLElement;
  busy: boolean;
};

/** The size and free-space lines of an estimate, separate so neither hides the other. */
function estimateLine(): EstimateLine {
  const size = h("span", { class: "estimate-size" });
  const detail = h("span", { class: "estimate-detail" });
  // Not a live region itself: it is rewritten for every step of a run and every frame of
  // the batch counts, which a screen reader would read out again and again.
  const text = h(
    "p",
    {
      class: "estimate",
      attrs: { title: "Estimated from what the site reports. Claws checks the actual free space again when each download starts." },
    },
    size,
    detail,
  );
  text.hidden = true;
  const status = h("span", { class: "visually-hidden", attrs: { role: "status", "aria-live": "polite" } });
  return { root: h("div", { class: "estimate-wrap" }, text, status), text, size, detail, status, busy: false };
}

function setText(el: HTMLElement, text: string) {
  if (el.textContent !== text) el.textContent = text;
}

/**
 * Paints an estimate. The free-space line keeps its height while it is empty, so the
 * Download button below does not move each time an estimate starts and settles. A screen
 * reader hears the outcome once it settles (and, with `announceStart`, that it started).
 */
function paintLine(line: EstimateLine, size: string, detail: string, busy = false, announceStart = false) {
  setText(line.size, size);
  setText(line.detail, detail || "\u00a0");
  line.detail.dataset.empty = String(!detail);
  line.text.hidden = !size;
  line.text.dataset.busy = String(busy);
  if (busy && !line.busy && announceStart && size) setText(line.status, size);
  else if (!busy && line.busy && size) setText(line.status, detail ? `${size}. ${detail}` : size);
  line.busy = busy;
}

/** Ignores clicks for this long after a run starts (the second click of a double click). */
const STOP_GUARD_MS = 600;

/**
 * The Estimate Sizes button of a list. It becomes Stop Estimating while a run goes; the
 * second click of a double click, or Enter held down, does not stop the run it started.
 */
function estimateButton(run: EstimateRun<EstimateItem>, start: () => void): HTMLButtonElement {
  let startedAt = -Infinity;
  const btn: HTMLButtonElement = button({
    label: "Estimate Sizes",
    variant: "plain",
    size: "small",
    onClick: () => {
      if (run.state.running) {
        if (performance.now() - startedAt >= STOP_GUARD_MS) run.cancel();
        return;
      }
      if (btn.getAttribute("aria-disabled") === "true") return;
      startedAt = performance.now();
      start();
    },
  });
  // Enter held down repeats clicks; only the first press counts.
  btn.addEventListener("keydown", (event) => {
    if (event.repeat && (event.key === "Enter" || event.key === " ")) event.preventDefault();
  });
  return btn;
}

/**
 * A run's key: the choices it was asked with. The items it covers are followed apart
 * (EstimateRun.follow), by the queue's identity for each item, not by position or row id.
 */
function runKey(options: DownloadOptions, settings: unknown[]): string {
  return JSON.stringify([options, settings]);
}

/**
 * Estimate calls running at once, across the single preview, a playlist's run and the
 * batch's run, and across views: a reset run's calls keep their slots until they answer.
 */
const estimateCalls = new CallLimit();

interface BatchRowRefs {
  pick: HTMLButtonElement;
  thumb: HTMLElement;
  thumbFor: string | null;
  title: HTMLElement;
  sub: HTMLElement;
  side: HTMLElement;
  sideFor: string;
}

export const mountDownload: MountView = (container, ctx) => {
  ctx.setActions([
    button({
      label: "Open Folder",
      icon: "folder",
      variant: "secondary",
      size: "small",
      onClick: () => void api.openOutputDir().catch((e) => toast(errorMessage(e), { tone: "error" })),
    }),
  ]);

  let phase: Phase = store.batch ? "batch" : "idle";
  let result: ProbeResult | null = null;
  let errorTitle = "Couldn't read that link";
  let errorText = "";
  let errorRetry = true;
  let lastUrl = "";
  /** Lines skipped beside the one link being looked up (several lines, one link in them). */
  let skipped: ParsedInput | null = null;
  /** Bumped by every submission; an older lookup's reply is dropped. */
  let ticket = 0;
  let stageAnimation: Animation | null = null;
  let batchView: { paintRow(row: BatchRow<ProbeResult>): void; refresh(): void; dispose(): void } | null = null;

  let mode: DownloadOptions["mode"] = "video";
  let height: number | null = null;
  let audioFormat = "mp3";
  let subs = new Set<string>();
  let embedSubs = false;
  let fastSave = false;
  let selected = new Set<number>();
  /** True while an enqueue is in flight or its confirmation is showing. */
  let enqueuing = false;
  /** Saved details shown while a fresh lookup runs ("checking"), or after it failed ("stale"). */
  let freshness: "fresh" | "checking" | "stale" = "fresh";
  let staleReason = "";
  let freshnessEl: HTMLElement | null = null;

  // ---- Size previews (display only; the download checks free space when it starts) ----
  /** One link: follows the choices, debounced, latest answer only. */
  let previewLine: EstimateLine | null = null;
  const preview = new PreviewEstimator(
    (key) => {
      const [url, options] = JSON.parse(key) as [string, DownloadOptions];
      return api.estimate(url, null, options);
    },
    { calls: estimateCalls, onChange: () => paintPreviewEstimate() },
  );
  const estimateItem = (item: EstimateItem) => api.estimate(item.url, item.playlistIndex, item.options);
  // Repeats are dropped by the queue's identity (enqueue adds each item once).
  const runOptions = { calls: estimateCalls, id: (item: EstimateItem) => itemKey(item) };
  /** A playlist's selected entries, only when asked ("Estimate Sizes"). */
  let paintListEstimate: (() => void) | null = null;
  const listRun = new EstimateRun<EstimateItem>(estimateItem, { ...runOptions, onChange: () => paintListEstimate?.() });
  /** The batch's selected links, only when asked. */
  let paintBatchEstimate: (() => void) | null = null;
  const batchRun = new EstimateRun<EstimateItem>(estimateItem, { ...runOptions, onChange: () => paintBatchEstimate?.() });

  // A text area that looks like a one-line field until it holds several lines: one link
  // per line makes a batch.
  const input = h("textarea", {
    class: "url-input",
    attrs: {
      rows: 1,
      wrap: "off",
      placeholder: "Paste a video link, or several on separate lines",
      "aria-label": "Video link, or several links on separate lines",
      "aria-keyshortcuts": "Control+Enter",
      autocomplete: "off",
      spellcheck: false,
    },
  });
  const goButton = button({ label: "Get Info", variant: "primary", onClick: () => void submit() });
  const goLabel = goButton.querySelector("span")!;
  const pasteButton = button({
    icon: "paste",
    title: "Paste from clipboard",
    variant: "plain",
    onClick: async () => {
      try {
        const text = (await navigator.clipboard.readText()).trim();
        if (text) {
          input.value = text;
          fitInput();
          input.focus();
          // A list waits for "Look Up"; a single link is looked up at once, as before.
          if (looksLikeUrl(text)) void submit();
        }
      } catch {
        toast("Clipboard access was blocked. Press Ctrl+V in the field instead.", { tone: "error" });
      }
    },
  });
  const urlBar = h("form", { class: "urlbar glass", attrs: { role: "search" } }, icon("link", 18), input, pasteButton, goButton);
  urlBar.addEventListener("submit", (event) => {
    event.preventDefault();
    void submit();
  });

  const multiline = () => /[\r\n]/.test(input.value.trim());
  /** Grows the field with its lines (up to six, then it scrolls) and names the button. */
  function fitInput() {
    const lines = input.value.replace(/\s+$/, "").split(/\r\n|\r|\n/).length;
    input.rows = Math.min(Math.max(lines, 1), 6);
    urlBar.classList.toggle("multiline", lines > 1);
    const count = lines > 1 ? parseLinks(input.value).links.length : 0;
    goLabel.textContent = count > 1 ? `Look Up ${count} Links` : "Get Info";
  }
  input.addEventListener("input", fitInput);
  // Enter looks up a single link. With several lines, Enter adds a line and Ctrl+Enter
  // looks them all up; Shift+Enter always adds a line.
  input.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.isComposing) return;
    if (event.ctrlKey || event.metaKey || (!event.shiftKey && !multiline())) {
      event.preventDefault();
      void submit();
    }
  });

  const setup = h("div", { class: "setup-slot" });
  const stage = h("div", { class: "stage" });
  container.append(urlBar, setup, stage);

  const paintSetup = () => {
    if (!store.toolsKnown) return clear(setup);
    const missing = !store.tools.ytdlp;
    const busy = Boolean(store.toolBusy.ytdlp);
    const error = store.toolError.ytdlp;
    if (!missing && !busy && !error) return clear(setup);

    const progress = store.toolProgress.ytdlp;
    const fraction = progress && progress.total > 0 ? progress.received / progress.total : null;
    const bar = progressBar(fraction);
    setProgress(bar, busy ? fraction : 0);
    replace(
      setup,
      h(
        "div",
        { class: `banner glass${error ? " banner-error" : ""}`, attrs: { role: "status" } },
        h(
          "div",
          { class: "banner-text" },
          h("strong", { text: error ? "Setup could not finish" : "Setting up Claws" }),
          h("span", {
            text: error
              ? error
              : "Downloading yt-dlp, the engine that reads each site. This happens once and takes a moment.",
          }),
          busy ? bar : null,
        ),
        error
          ? button({ label: "Try Again", icon: "retry", variant: "primary", onClick: () => void installEngine() })
          : null,
      ),
    );
  };
  const installEngine = () => store.runTool("ytdlp", api.installYtdlp);
  const offTools = [store.on("tools", paintSetup), store.on("toolProgress", paintSetup)];
  paintSetup();
  void store.toolsReady.then(() => {
    if (!store.tools.ytdlp && !store.toolBusy.ytdlp && !store.toolError.ytdlp) void installEngine();
  });

  /** Shows a lookup result with every option back at its default. */
  function showResult(next: ProbeResult) {
    result = next;
    mode = "video";
    height = null;
    audioFormat = "mp3";
    subs = new Set();
    embedSubs = false;
    fastSave = false;
    selected = new Set(next.entries.map((_, i) => i));
    phase = "result";
    preview.cancel();
    listRun.reset();
  }

  /** When this is unchanged, a refresh redraws nothing. */
  const shape = (r: ProbeResult) =>
    JSON.stringify([r.title, r.thumbnail, r.duration, r.uploader, r.heights, r.subtitles.map((s) => s.code), r.entries.map((e) => e.url + e.title)]);

  /** Takes refreshed details while keeping the choices already made. */
  function adoptRefresh(next: ProbeResult) {
    const before = result;
    result = next;
    subs = new Set([...subs].filter((code) => next.subtitles.some((s) => s.code === code)));
    if (!before || before.entries.length !== next.entries.length) selected = new Set(next.entries.map((_, i) => i));
    // Sizes estimated for other videos do not describe these.
    const urls = (r: ProbeResult) => JSON.stringify(r.entries.map((e) => [e.url, e.playlistIndex]));
    if (!before || urls(before) !== urls(next)) listRun.reset();
  }

  function showError(title: string, text: string, retry = true) {
    phase = "error";
    errorTitle = title;
    errorText = text;
    errorRetry = retry;
    render();
  }

  async function submit() {
    const text = input.value.trim();
    if (!text) {
      input.focus();
      return;
    }
    if (multiline()) {
      const parsed = parseLinks(text);
      if (isBatchInput(parsed)) return submitBatch(parsed);
      // The same link on several lines is still one link, and one link beside lines that
      // are not links gets the ordinary preview, which names the skipped lines.
      if (parsed.links.length === 1) return lookUp(parsed.links[0].url, parsed.invalid.length > 0 ? parsed : null);
    }
    return lookUp(text);
  }

  // A batch is never discarded by another lookup or by input that was refused: it stays in
  // the store, one "Back to Links" press away (see batchReturn). Only Clear, a new list
  // that is actually looked up, or adding every row ends it.
  async function lookUp(url: string, issues: ParsedInput | null = null) {
    const mine = ++ticket;
    skipped = issues;
    if (!looksLikeUrl(url)) {
      lastUrl = "";
      return showError("Couldn't read that link", "That does not look like a link. Paste the full address, starting with https://");
    }
    await store.toolsReady;
    if (mine !== ticket) return;
    if (!store.tools.ytdlp) {
      lastUrl = "";
      return showError("Couldn't read that link", "yt-dlp is still being set up. Try again in a moment.");
    }
    lastUrl = url;

    // Saved details show at once; the lookup below checks them.
    const cached = await api.cachedLink(url).catch(() => null);
    if (mine !== ticket) return; // a newer request replaced this one
    if (cached) {
      showResult(cached.result);
      freshness = "checking";
    } else {
      phase = "loading";
      freshness = "fresh";
    }
    render();

    try {
      const probed = await api.probe(url);
      if (mine !== ticket) return;
      if (cached && phase === "result" && result) {
        const changed = shape(result) !== shape(probed);
        freshness = "fresh";
        if (changed) {
          adoptRefresh(probed);
          render();
        } else {
          result = probed;
          paintFreshness();
          scheduleEstimate(); // the lookup is cached now, so the preview needs no network
        }
        return;
      }
      showResult(probed);
    } catch (error) {
      if (mine !== ticket) return;
      if (cached && phase === "result") {
        // Keep the saved details on screen, but say they could not be confirmed.
        freshness = "stale";
        staleReason = errorMessage(error);
        paintFreshness();
        // The cached details may still answer offline; if not, the line says Size unknown.
        scheduleEstimate();
        return;
      }
      phase = "error";
      errorTitle = "Couldn't read that link";
      errorText = errorMessage(error);
      errorRetry = true;
    }
    render();
  }

  /** Several links: each is looked up (three at a time); nothing downloads until asked. */
  async function submitBatch(parsed: ParsedInput) {
    const mine = ++ticket;
    lastUrl = "";
    skipped = null;
    if (parsed.overLimit) {
      return showError(
        "Too many links",
        `That is ${parsed.links.length} different links. Claws looks up at most ${MAX_LINKS} at once: paste the first ${MAX_LINKS}, add them to the queue, then paste the rest.`,
        false,
      );
    }
    if (parsed.links.length === 0) {
      const first = parsed.invalid[0];
      return showError("No links found", first ? `Line ${first.line}: ${first.reason}` : "Paste one http(s) link on each line.", false);
    }
    await store.toolsReady;
    if (mine !== ticket) return;
    if (!store.tools.ytdlp) return showError("Couldn't read those links", "yt-dlp is still being set up. Try again in a moment.");
    input.value = "";
    fitInput();
    phase = "batch";
    store.startBatch(parsed); // its "batch" event draws the panel
  }

  function openLink(url: string) {
    input.value = url;
    fitInput();
    void submit();
  }

  function currentOptions(): DownloadOptions {
    const chosenSubs = [...subs];
    const hasAuto = result?.subtitles.some((s) => s.auto && subs.has(s.code)) ?? false;
    return {
      mode,
      maxHeight: mode === "video" ? height : null,
      audioFormat,
      subtitleLangs: mode === "video" ? chosenSubs : [],
      autoSubs: hasAuto,
      embedSubs,
      fastSave,
    };
  }

  /**
   * Asks for the preview that matches the current choices once the lookup has settled:
   * current details let the backend estimate without reading the site again. When the
   * check failed (stale) it still asks: the backend uses the cached details if they are
   * still kept, and otherwise answers with an unknown size and its reason.
   */
  function scheduleEstimate() {
    paintListEstimate?.();
    if (!result || result.kind !== "video" || freshness === "checking") return;
    // The settings the backend reads are part of the key, so a change of them asks again.
    preview.request(JSON.stringify([result.url, currentOptions(), ...estimateSettings()]));
  }

  function paintPreviewEstimate() {
    if (!previewLine) return;
    const { size, detail } = describeSingle(preview.state);
    paintLine(previewLine, size, detail, preview.state.status === "loading");
  }

  /** The line under a list, and its button, for a run of estimates. */
  function paintRun(run: EstimateRun<EstimateItem>, line: EstimateLine, btn: HTMLButtonElement, count: number) {
    const s = run.state;
    const finished = run.key !== null && !s.running && s.done > 0;
    paintLine(line, describeRun(run), finished ? (aggregate(s.results).tempText ?? "") : "", s.running, true);
    // Named apart from the batch's own Stop (which stops reading links).
    setText(btn.querySelector("span")!, s.running ? "Stop Estimating" : "Estimate Sizes");
    const title = s.running
      ? "Stops estimating. Sizes already read stay; estimates already running finish in the background and are ignored."
      : `Reads the size of each selected video, up to ${run.maxItems}, three at a time. Nothing is read until you ask.`;
    if (btn.title !== title) btn.title = title;
    btn.setAttribute("aria-disabled", String(!s.running && count === 0));
  }

  /** The settings the backend reads for an estimate; a change of them asks again. */
  const estimateSettings = () => [store.settings.audioSelection, store.settings.embedMetadata];

  async function startDownload(trigger?: HTMLButtonElement) {
    // A double click, or a click during the confirmation, would queue it twice.
    if (!result || enqueuing) return;
    enqueuing = true;
    const mine = ticket;
    if (trigger) trigger.disabled = true;
    const items =
      result.kind === "playlist"
        ? result.entries
            .filter((_, i) => selected.has(i))
            .map((e) => ({
              url: e.url,
              playlistIndex: e.playlistIndex,
              title: e.title,
              thumbnail: e.thumbnail,
              uploader: e.uploader,
              duration: e.duration,
            }))
        : [
            {
              url: result.url,
              title: result.title,
              thumbnail: result.thumbnail,
              uploader: result.uploader,
              duration: result.duration,
            },
          ];
    try {
      // The looked-up link, so its details are saved in the link cache.
      const added = await api.enqueue(items, currentOptions(), lastUrl || result.url);
      // Usually already applied from the jobs-added event; the revisions make this a no-op.
      store.addJobs(added);
      const jobs = added.jobs;
      if (trigger) {
        trigger.classList.add("confirmed");
        replace(trigger, icon("check", 16), h("span", { text: jobs.length === 1 ? "Added" : `${jobs.length} Added` }));
        await new Promise((resolve) => setTimeout(resolve, 650));
      }
      toast(jobs.length === 1 ? "Added to the queue." : `${jobs.length} videos added to the queue.`, {
        action: { label: "View Queue", run: () => store.go("queue") },
      });
      if (mine !== ticket) return; // another link was submitted meanwhile
      input.value = "";
      fitInput();
      lastUrl = "";
      result = null;
      // Back to the list this link was looked up over, if there is one.
      phase = store.batch ? "batch" : "idle";
      render();
    } catch (error) {
      toast(errorMessage(error), { tone: "error" });
      if (trigger) trigger.disabled = false;
    } finally {
      enqueuing = false;
    }
  }

  function thumbnailEl(url: string | null, cls: string): HTMLElement {
    if (!url) return h("div", { class: `${cls} thumb-fallback` }, icon("video", 22));
    const img = h("img", { class: cls, attrs: { src: url, alt: "", loading: "lazy", decoding: "async", referrerpolicy: "no-referrer", draggable: false } });
    img.addEventListener("error", () => img.replaceWith(h("div", { class: `${cls} thumb-fallback` }, icon("video", 22))));
    return img;
  }

  function renderPreview(r: ProbeResult): HTMLElement {
    const art = r.thumbnail ?? r.entries.find((e) => e.thumbnail)?.thumbnail ?? null;
    const meta = [
      r.kind === "playlist" ? `${r.entries.length} videos` : formatDuration(r.duration),
      r.uploader,
      r.site,
    ].filter(Boolean) as string[];
    freshnessEl = h("p", { class: "preview-note", attrs: { "aria-live": "polite" } });
    const section = h(
      "section",
      { class: "preview glass", attrs: { "aria-label": "Link details" } },
      thumbnailEl(art, "preview-thumb"),
      h(
        "div",
        { class: "preview-info" },
        h("h2", { class: "preview-title", text: r.title }),
        h("p", { class: "preview-meta", text: meta.join("  ·  ") }),
        freshnessEl,
      ),
    );
    paintFreshness();
    return section;
  }

  function paintFreshness() {
    if (!freshnessEl || !result) return;
    const r = result;
    const previous = store.history.find((item) => item.url === r.url || item.url === lastUrl);
    const parts = [
      previous ? `Downloaded ${formatDate(previous.finishedAt)}` : null,
      freshness === "checking" ? "Checking for changes…" : null,
      freshness === "stale" ? `Couldn't check for changes: ${staleReason}` : null,
    ].filter(Boolean) as string[];
    freshnessEl.textContent = parts.join("  ·  ");
    freshnessEl.hidden = parts.length === 0;
    freshnessEl.dataset.tone = freshness === "stale" ? "warn" : "";
  }

  function renderEntries(r: ProbeResult): HTMLElement {
    const countLabel = h("span", { class: "entries-count" });
    const list = h("div", { class: "entries", attrs: { role: "group", "aria-label": "Videos in this playlist" } });
    // Created with a label: an icon-only button is a fixed 28 px wide.
    const downloadBtn: HTMLButtonElement = button({ label: "Download", icon: "download", variant: "primary", onClick: () => void startDownload(downloadBtn) });
    const listLine = estimateLine();
    const listItems = (): EstimateItem[] => {
      const options = currentOptions();
      return r.entries.filter((_, i) => selected.has(i)).map((e) => ({ url: e.url, playlistIndex: e.playlistIndex, options }));
    };
    // Only the choices: the entries are followed by link and position (EstimateRun.follow),
    // so a refreshed list with other videos at the same positions does not keep the old
    // videos' sizes.
    const listKey = () => runKey(currentOptions(), estimateSettings());
    const estimateBtn = estimateButton(listRun, () => listRun.start(listItems(), listKey()));
    paintListEstimate = () => {
      // Other choices, or an estimated entry unticked: the estimate no longer applies (and
      // follow resets it, which repaints). An entry ticked since is named as not included.
      if (!listRun.follow(listKey(), listItems())) return;
      paintRun(listRun, listLine, estimateBtn, selected.size);
    };

    const paintCount = () => {
      paintListEstimate?.();
      countLabel.textContent = `${selected.size} of ${r.entries.length} selected`;
      const label = selected.size === 1 ? "Download 1 Video" : `Download ${selected.size} Videos`;
      replace(downloadBtn, icon("download", 16), h("span", { text: selected.size === 0 ? "Download" : label }));
      downloadBtn.disabled = selected.size === 0;
    };

    const toggleAll = button({
      label: selected.size === r.entries.length ? "Select None" : "Select All",
      variant: "plain",
      size: "small",
      onClick: () => {
        selected = selected.size === r.entries.length ? new Set() : new Set(r.entries.map((_, i) => i));
        toggleAll.querySelector("span")!.textContent = selected.size === r.entries.length ? "Select None" : "Select All";
        list.querySelectorAll<HTMLButtonElement>(".entry").forEach((el, i) => el.setAttribute("aria-checked", String(selected.has(i))));
        paintCount();
      },
    });

    r.entries.forEach((entry, index) => {
      const item = h(
        "button",
        { class: "entry", attrs: { type: "button", role: "checkbox", "aria-checked": String(selected.has(index)) } },
        h("span", { class: "check" }, icon("check", 12)),
        thumbnailEl(entry.thumbnail, "entry-thumb"),
        h(
          "span",
          { class: "entry-text" },
          h("span", { class: "entry-title", text: entry.title }),
          entry.uploader ? h("span", { class: "entry-sub", text: entry.uploader }) : null,
        ),
        h("span", { class: "entry-duration", text: formatDuration(entry.duration) }),
      );
      item.addEventListener("click", () => {
        if (selected.has(index)) selected.delete(index);
        else selected.add(index);
        item.setAttribute("aria-checked", String(selected.has(index)));
        toggleAll.querySelector("span")!.textContent = selected.size === r.entries.length ? "Select None" : "Select All";
        paintCount();
      });
      list.appendChild(item);
    });
    paintCount();
    return h(
      "div",
      { class: "entries-block" },
      h("div", { class: "entries-head" }, sectionTitle("Videos"), countLabel, estimateBtn, toggleAll),
      h("div", { class: "glass group" }, list),
      listLine.root,
      h("div", { class: "actions" }, h("span", { class: "save-to", text: `Saves to ${folderName(store.settings.outputDir)}` }), downloadBtn),
    );
  }

  function renderOptions(r: ProbeResult): HTMLElement {
    const playlist = r.kind === "playlist";
    const qualities = qualityChoices(r.heights, playlist);
    const body = h("div", { class: "options-body" });

    const paint = () => {
      const rows: (Node | null)[] = [];
      const typeSeg = segmented<"video" | "audio">(
        "Type",
        [
          { value: "video", label: "Video", icon: "video" },
          { value: "audio", label: "Audio Only", icon: "music" },
        ],
        mode,
        (next) => {
          mode = next;
          paint();
        },
      );
      rows.push(row("Type", typeSeg.el));

      if (mode === "video") {
        rows.push(
          row(
            "Quality",
            chipRadio<number>(
              "Quality",
              [{ value: 0, label: "Best" }, ...qualities.map((q) => ({ value: q.value, label: q.label, hint: `Up to ${q.value}p` }))],
              height ?? 0,
              (value) => {
                height = value === 0 ? null : value;
                scheduleEstimate();
              },
            ),
            qualities.length === 0
              ? "Quality options appear once the site reports them."
              : playlist
                ? "Each video is saved at the best quality it has, up to this limit. MP4 when possible."
                : "Saved as MP4 when possible.",
          ),
        );
        if (r.subtitles.length > 0) {
          rows.push(
            row(
              "Subtitles",
              chipCheck(
                "Subtitles",
                r.subtitles.slice(0, 14).map((s) => ({ value: s.code, label: s.name })),
                subs,
                () => paint(),
              ),
            ),
          );
          if (subs.size > 0) {
            rows.push(
              row(
                "Embed in video",
                toggle("Embed subtitles in the video", embedSubs, (v) => {
                  embedSubs = v;
                  paint();
                }),
                "Off saves them as separate .srt files.",
              ),
            );
          }
        }
      } else {
        rows.push(
          row(
            "Format",
            chipRadio<string>("Audio format", AUDIO_FORMATS, audioFormat, (value) => {
              audioFormat = value;
              scheduleEstimate();
            }),
            audioHint(),
          ),
        );
      }
      rows.push(fastSaveRow());
      replace(body, group(...rows));
      scheduleEstimate();
    };

    // Fast Save names exactly what it changes for these choices; it never changes the
    // video or audio that is downloaded.
    const fastSaveRow = () => {
      const metadata = store.settings.embedMetadata;
      const embeddedSubs = mode === "video" && subs.size > 0 && embedSubs;
      const skipped = [metadata ? "writing title, uploader and date into the file" : null, embeddedSubs ? "embedding subtitles (they are saved as .srt files instead)" : null].filter(Boolean);
      const hint = skipped.length === 0
        ? "Nothing to skip with these choices: the file is not rewritten after it downloads."
        : fastSave
          ? `Skips ${skipped.join(" and ")}. The video and audio are the same.`
          : metadata && embeddedSubs
            ? "Embedding metadata and subtitles each rewrite the whole file after it downloads. Fast Save skips both."
            : `Skips ${skipped.join(" and ")}, which rewrites the whole file after it downloads.`;
      return row(
        "Fast Save",
        toggle("Fast Save", fastSave, (v) => {
          fastSave = v;
          paint();
        }),
        hint,
      );
    };
    paint();
    return body;
  }

  function renderResult(r: ProbeResult): HTMLElement {
    const wrap = h("div", { class: "result" }, renderPreview(r));
    if (r.kind === "playlist" && r.entries.length === 0) {
      wrap.append(h("div", { class: "notice" }, icon("alert", 16), h("span", { text: "This playlist has no downloadable videos." })));
      return wrap;
    }
    wrap.append(sectionTitle("Options"), renderOptions(r));
    if (r.kind === "playlist") {
      wrap.append(renderEntries(r));
    } else {
      previewLine = estimateLine();
      paintPreviewEstimate();
      wrap.append(
        previewLine.root,
        h(
          "div",
          { class: "actions" },
          h("span", { class: "save-to", text: `Saves to ${folderName(store.settings.outputDir)}` }),
          (() => {
            const btn: HTMLButtonElement = button({ label: "Download", icon: "download", variant: "primary", onClick: () => void startDownload(btn) });
            return btn;
          })(),
        ),
      );
    }
    return wrap;
  }

  // ---- Several links ---------------------------------------------------------------

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
    batchView?.refresh(); // disables the button while the rows are sent
    const summary = await sending;
    batchView?.refresh();
    if (!summary) return;
    toast(describeEnqueue(summary), {
      tone: summary.added === 0 && summary.failed > 0 ? "error" : "info",
      action: summary.added > 0 ? { label: "View Queue", run: () => store.go("queue") } : undefined,
    });
    // Everything is queued: back to the empty page, with the keyboard in the link field.
    // Otherwise the rest stays, to retry.
    if (store.batch === session && store.links.rows.every((r) => r.added)) {
      const active = document.activeElement;
      const wasHere = !active || active === document.body || stage.contains(active);
      store.clearBatch();
      if (wasHere) input.focus({ preventScroll: true });
    }
  }

  function rowNote(r: BatchRow<ProbeResult>): [string, string] {
    // Created now, or already waiting there with the same choices: either way it is queued.
    if (r.added) return ["In the queue", "done"];
    switch (r.status) {
      case "waiting":
        return ["Waiting to be read", ""];
      case "looking":
        return ["Reading the link…", ""];
      case "failed":
        return [r.error ?? "Couldn't read this link", "error"];
      case "cancelled":
        return ["Stopped before it was read", ""];
      case "ready": {
        if (r.addError) return [`Couldn't add: ${r.addError}`, "error"];
        const res = r.result!;
        const count = res.entries.length;
        const lead =
          res.kind === "playlist"
            ? count === 0
              ? "Playlist · no downloadable videos"
              : `Playlist · ${count} ${count === 1 ? "item" : "items"}`
            : formatDuration(res.duration);
        return [[lead, res.uploader, res.site].filter(Boolean).join("  ·  "), ""];
      }
    }
  }

  function renderIssues(parsed: ParsedInput): HTMLElement | null {
    const lines = [
      ...parsed.invalid.map((i) => ({ line: i.line, text: `Line ${i.line}, “${shorten(i.text)}”: ${i.reason}` })),
      ...parsed.duplicates.map((d) => ({ line: d.line, text: `Line ${d.line} repeats line ${d.firstLine}; it is read once.` })),
    ].sort((a, b) => a.line - b.line);
    if (lines.length === 0) return null;
    return h(
      "div",
      { class: "notice batch-issues" },
      icon("alert", 16),
      h(
        "div",
        { class: "notice-text" },
        h("strong", { text: lines.length === 1 ? "1 line was skipped" : `${lines.length} lines were skipped` }),
        h("ul", {}, ...lines.slice(0, ISSUES_SHOWN).map((l) => h("li", { text: l.text }))),
        lines.length > ISSUES_SHOWN ? h("span", { text: `And ${lines.length - ISSUES_SHOWN} more.` }) : null,
      ),
    );
  }

  /**
   * The batch: one row per link with its own status, built ROWS_PER_FRAME rows a frame.
   * A row change repaints that row only; the counts repaint at most once a frame.
   */
  function renderBatch(session: BatchSession): HTMLElement {
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
    const batchLine = estimateLine();
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
    paintBatchEstimate = () => {
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
      paintBatchEstimate?.();
    };
    const scheduleCounts = () => {
      if (!countFrame) countFrame = requestAnimationFrame(paintCounts);
    };

    const fallbackThumb = () => h("div", { class: "entry-thumb thumb-fallback" }, icon("link", 16));

    const paintRow = (r: BatchRow<ProbeResult>) => {
      scheduleCounts();
      const ref = refs.get(r.id);
      if (!ref) return; // not built yet; it is painted when it is
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
                  ref.pick.focus(); // the button goes away; focus stays on the row
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
    const qualities = qualityChoices([], true);
    const qualityRow = row(
      "Quality",
      chipRadio<number>(
        "Quality",
        [{ value: 0, label: "Best" }, ...qualities.map((q) => ({ value: q.value, label: q.label, hint: `Up to ${q.value}p` }))],
        choices.maxHeight ?? 0,
        (value) => {
          choices.maxHeight = value === 0 ? null : value;
          scheduleCounts(); // an estimate for other choices no longer applies
        },
      ),
      "Each video is saved at the best quality it has, up to this limit. MP4 when possible.",
    );
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

    batchView = {
      paintRow,
      refresh: scheduleCounts,
      dispose: () => {
        cancelAnimationFrame(buildFrame);
        cancelAnimationFrame(countFrame);
      },
    };
    paintCounts();

    return h(
      "div",
      { class: "result batch" },
      renderIssues(session.parsed),
      sectionTitle("Options"),
      group(
        row("Type", typeSeg.el, "Applies to every link here. Subtitles can be chosen when a link is read on its own."),
        qualityRow,
        formatRow,
      ),
      h("div", { class: "entries-head" }, sectionTitle("Links"), counts, announcer, stopBtn, retryBtn, estimateBtn, toggleAll, clearBtn),
      h("div", { class: "glass group" }, list),
      batchLine.root,
      h("div", { class: "actions" }, selectedNote, addBtn),
    );
  }

  function renderRecent(): Node | null {
    const recent = store.history.slice(0, 4);
    if (recent.length === 0) return null;
    return h(
      "section",
      { class: "recent" },
      h("div", { class: "recent-head" }, sectionTitle("Recent"), button({ label: "Show Library", variant: "plain", size: "small", onClick: () => store.go("library") })),
      group(
        ...recent.map((item) =>
          h(
            "div",
            { class: "row recent-row" },
            h(
              "button",
              {
                class: "recent-open",
                attrs: { type: "button", title: "Show the details again" },
                on: { click: () => openLink(item.url) },
              },
              thumbnailEl(item.thumbnailLocal ? convertFileSrc(item.thumbnailLocal) : item.thumbnail, "recent-thumb"),
              h("span", { class: "recent-text" }, h("span", { class: "row-title", text: item.title }), h("span", { class: "row-hint", text: [item.quality, item.uploader].filter(Boolean).join("  ·  ") })),
            ),
            button({
              icon: "folder",
              title: "Show in folder",
              variant: "plain",
              size: "small",
              onClick: () => void api.revealDownload(item.id).catch((e) => toast(errorMessage(e), { tone: "error" })),
            }),
          ),
        ),
      ),
    );
  }

  /** Shown above a single link's details, or an error, while a batch is kept behind them. */
  function batchReturn(): HTMLElement | null {
    if (!store.batch || phase === "batch") return null;
    const total = store.links.rows.length;
    const waiting = store.links.rows.filter((r) => !r.added).length;
    return h(
      "div",
      { class: "batch-return" },
      h("span", { text: `Your list of ${total} ${total === 1 ? "link" : "links"} is kept${waiting > 0 ? `, ${waiting} not added yet` : ""}.` }),
      button({
        label: "Back to Links",
        variant: "secondary",
        size: "small",
        onClick: () => {
          ticket += 1; // a single lookup still running is dropped
          lastUrl = "";
          result = null;
          phase = "batch";
          render();
        },
      }),
    );
  }

  /** What the stage shows; a change of it (not a refresh of the same) animates in. */
  const stageKey = () =>
    phase === "result" ? `result:${lastUrl}` : phase === "batch" ? `batch:${store.links.rows[0]?.id ?? ""}` : phase;
  let shownKey = "";

  function render() {
    goButton.disabled = phase === "loading";
    // A newer transition replaces one still running; errors and loading never wait for one.
    stageAnimation?.cancel();
    stageAnimation = null;
    batchView?.dispose();
    batchView = null;
    // The lines are rebuilt below; what they showed is kept only where it still applies.
    previewLine = null;
    paintListEstimate = null;
    paintBatchEstimate = null;
    if (phase !== "result") {
      preview.cancel();
      listRun.reset();
    }
    if (phase !== "batch") batchRun.reset();
    const hadFocus = stage.contains(document.activeElement);
    const key = stageKey();
    const changed = key !== shownKey;
    shownKey = key;
    const back = batchReturn();
    // Shown with the single link's loading, error and details; never with a batch.
    const issues = skipped && phase !== "idle" && phase !== "batch" ? renderIssues(skipped) : null;
    switch (phase) {
      case "loading":
        replace(stage, back, issues, h("div", { class: "loading", attrs: { role: "status" } }, spinner(), h("span", { text: "Reading the link…" })));
        break;
      case "error":
        replace(
          stage,
          back,
          issues,
          h(
            "div",
            { class: "notice notice-error", attrs: { role: "alert" } },
            icon("alert", 18),
            h("div", { class: "notice-text" }, h("strong", { text: errorTitle }), h("span", { text: errorText })),
            errorRetry ? button({ label: "Try Again", variant: "secondary", size: "small", onClick: () => void submit() }) : null,
          ),
        );
        break;
      case "result": {
        const shown = renderResult(result!);
        replace(stage, back, issues, shown);
        if (changed) stageAnimation = settleIn(shown);
        break;
      }
      case "batch":
        if (!store.batch) {
          phase = "idle";
          return render();
        }
        replace(stage, renderBatch(store.batch));
        if (changed) stageAnimation = settleIn(stage.firstElementChild);
        break;
      default:
        replace(
          stage,
          back,
          h(
            "div",
            { class: "empty" },
            h("div", { class: "empty-symbol" }, icon("download", 46)),
            h("h2", { class: "empty-title", text: "Paste a link to get started" }),
            h("p", {
              class: "empty-text",
              text: "Claws saves video and audio from over a thousand sites. Copy a link, then press Ctrl+V anywhere in this window.",
            }),
          ),
          renderRecent(),
        );
    }
    // The focused control was replaced: keep the keyboard in the link field, not lost.
    if (hadFocus && !stage.contains(document.activeElement)) input.focus({ preventScroll: true });
  }

  // Ctrl+V anywhere in the window lands here (routed by main.ts).
  // A list of links is only filled in: reading 500 links is not started by a paste.
  const takePaste = () => {
    const pasted = store.takePastedLink();
    if (!pasted) return;
    input.value = pasted;
    fitInput();
    if (looksLikeUrl(pasted)) void submit();
    else input.focus();
  };
  const offPaste = store.on("paste", takePaste);
  const offFocus = store.on("focusLink", () => input.focus());
  const offHistory = store.on("history", () => phase === "idle" && render());
  const offBatch = store.on("batch", () => {
    if (store.batch) phase = "batch";
    else if (phase === "batch") phase = "idle";
    else return;
    render();
  });
  const offBatchRow = store.on("batchRow", (detail) => batchView?.paintRow(detail as BatchRow<ProbeResult>));

  render();
  input.focus();
  takePaste();

  return () => {
    offPaste();
    offFocus();
    offTools.forEach((off) => off());
    offHistory();
    offBatch();
    offBatchRow();
    batchView?.dispose();
    batchView = null;
    stageAnimation?.cancel();
    // Estimates still running answer into nothing.
    previewLine = null;
    paintListEstimate = null;
    paintBatchEstimate = null;
    preview.cancel();
    listRun.reset();
    batchRun.reset();
    ticket += 1; // replies to lookups still running are ignored
    lastUrl = "";
  };
};
