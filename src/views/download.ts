import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type DownloadMode, type DownloadOptions, type ProbeResult, type TranscriptFormat, type VideoPreset } from "../api";
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
import { fastSaveHint } from "../processing-options";
import { VIDEO_PRESETS, capChoices, hasExactSizes, keptQuality, qualityChoices } from "../resolution";
import {
  TRANSCRIPT_FORMATS,
  captionChoices,
  captionKey,
  defaultCaption,
  refreshSignature,
  refreshTrackChoices,
  subtitleLimitHint,
  transcriptOptions,
  type CaptionChoices,
} from "../transcript-options";
import { CHECK_INTERRUPTED, estimatesStillApply } from "../preview-session";
import { store, type BatchChoices, type BatchSession, type EstimateItem } from "../store";
import { button, chipRadio, segmented, toast, toggle } from "../ui/components";
import { append, clear, h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { group, progressBar, row, sectionTitle, setProgress, spinner } from "../ui/layout";
import { settleIn } from "../ui/motion";
import { subtitleChips } from "../ui/subtitle-chips";
import type { MountView } from "./shell";
import { SpotifyMatchController, isSpotifyInput } from "../spotify-match";
import { mountSpotifyMatch, type SpotifyMatchView } from "./spotify-match";

type Phase = "idle" | "loading" | "result" | "error" | "batch" | "spotify";

/** Batch rows built per frame, as in the Queue view: about a frame's worth of work. */
const ROWS_PER_FRAME = 40;
/** Skipped lines listed by name; the rest are counted. */
const ISSUES_SHOWN = 6;

const AUDIO_FORMATS = [
  { value: "mp3", label: "MP3" },
  { value: "m4a", label: "M4A" },
  { value: "opus", label: "Opus" },
  { value: "flac", label: "FLAC" },
  { value: "wav", label: "WAV" },
  { value: "best", label: "Original" },
];

/**
 * The video Format choice (Compatible or Original), with a hint that follows it. Updated in
 * place, so the control in use keeps focus.
 */
function presetRow(value: VideoPreset, onChange: (next: VideoPreset) => void): HTMLElement {
  const describe = (preset: VideoPreset) => VIDEO_PRESETS.find((p) => p.value === preset)?.hint ?? "";
  const hint = h("span", { text: describe(value) });
  const control = segmented<(typeof VIDEO_PRESETS)[number]["value"]>(
    "Video format",
    VIDEO_PRESETS.map((p) => ({ value: p.value, label: p.label })),
    value === "original" ? "original" : "compatible",
    (next) => {
      hint.textContent = describe(next);
      onChange(next);
    },
  );
  return row("Format", control.el, hint);
}

/** Why the Transcript type cannot be chosen for this video. */
const NO_CAPTIONS = "No captions available for this video";
/** Shown where a size estimate would be: a transcript downloads no media. */
const TEXT_ONLY = "Text only — the video is not downloaded";

/** The caption track for a transcript, human and automatic tracks in their own groups. */
function captionSelect(choices: CaptionChoices, value: string | null, onChange: (key: string) => void): HTMLSelectElement {
  const optgroup = (label: string, tracks: CaptionChoices["human"]) =>
    tracks.length === 0
      ? null
      : h("optgroup", { attrs: { label } }, ...tracks.map((t) => h("option", { attrs: { value: captionKey(t) }, text: t.name })));
  const select = h(
    "select",
    { class: "select-input", attrs: { "aria-label": "Caption language" } },
    optgroup("Human captions", choices.human),
    optgroup("Automatic captions", choices.automatic),
  );
  if (value) select.value = value;
  select.addEventListener("change", () => onChange(select.value));
  return select;
}

/** The Quality chips: Best, then each choice with its cap as the hint. */
function qualityChips(choices: { value: number; label: string; hint: string }[], value: number | null, onChange: (next: number | null) => void): HTMLElement {
  return chipRadio<number>("Quality", [{ value: 0, label: "Best" }, ...choices], value ?? 0, (next) => onChange(next === 0 ? null : next));
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
    videoPreset: choices.mode === "video" ? choices.videoPreset : "compatible",
    audioFormat: choices.audioFormat,
    subtitleLangs: [],
    autoSubs: false,
    embedSubs: false,
    fastSave: false,
    transcriptFormat: "txt",
  };
}

type EstimateLine = {
  root: HTMLElement;
  text: HTMLElement;
  size: HTMLElement;
  detail: HTMLElement;
  /** Visually hidden; written only when an estimate starts or settles, never per step. */
  status: HTMLElement;
  /** Asks again for what failed; shown only then. It never blocks Download. */
  retry: HTMLButtonElement;
  busy: boolean;
};

/**
 * The size and free-space lines of an estimate, separate so neither hides the other, and the
 * button that asks again for an estimate that failed (`onRetry`; the estimate state itself
 * enforces one retry at a time through the shared call limit).
 */
function estimateLine(onRetry: () => void, retryLabel = "Retry", retryTitle = "Asks again for this size. Download is not affected."): EstimateLine {
  const size = h("span", { class: "estimate-size" });
  const detail = h("span", { class: "estimate-detail" });
  const retry = button({
    label: retryLabel,
    variant: "plain",
    size: "small",
    title: retryTitle,
    onClick: () => {
      onRetry();
      // The button hides while the retry runs; keep keyboard focus beside it. The paragraph
      // keeps the global :focus-visible ring, so a keyboard user still sees where focus is.
      text.focus({ preventScroll: true });
    },
  });
  retry.hidden = true;
  retry.style.alignSelf = "flex-start";
  // Not a live region itself: it is rewritten for every step of a run and every frame of
  // the batch counts, which a screen reader would read out again and again.
  const text = h(
    "p",
    {
      class: "estimate",
      attrs: {
        title: "Estimated from what the site reports. Claws checks the actual free space again when each download starts.",
        tabindex: "-1",
      },
    },
    size,
    detail,
    retry,
  );
  text.hidden = true;
  const status = h("span", { class: "visually-hidden", attrs: { role: "status", "aria-live": "polite" } });
  return { root: h("div", { class: "estimate-wrap" }, text, status), text, size, detail, status, retry, busy: false };
}

function setText(el: HTMLElement, text: string) {
  if (el.textContent !== text) el.textContent = text;
}

/**
 * Paints an estimate. The free-space line keeps its height while it is empty, so the
 * Download button below does not move each time an estimate starts and settles. A screen
 * reader hears the outcome once it settles (and, with `announceStart`, that it started).
 */
function paintLine(line: EstimateLine, size: string, detail: string, busy = false, announceStart = false, canRetry = false) {
  line.retry.hidden = !canRetry;
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

  let mode: DownloadMode = "video";
  let height: number | null = null;
  let videoPreset: VideoPreset = "compatible";
  let audioFormat = "mp3";
  let subs = new Set<string>();
  let embedSubs = false;
  let fastSave = false;
  /** The caption track a transcript is made from (transcript-options.ts captionKey). */
  let caption: string | null = null;
  let transcriptFormat: TranscriptFormat = "txt";
  /** Names the Download button after the type (a transcript is saved, not downloaded). */
  let paintActionLabel: (() => void) | null = null;
  let selected = new Set<number>();
  /** True while an enqueue is in flight or its confirmation is showing. */
  let enqueuing = false;
  let activeEnqueue: symbol | null = null;
  /** Saved details shown while a fresh lookup runs ("checking"), or after it failed ("stale"). */
  let freshness: "fresh" | "checking" | "stale" = "fresh";
  let staleReason = "";
  /** What a refresh could not keep of the choices made (transcript-options.ts refreshTrackChoices). */
  let refreshNotices: string[] = [];
  let freshnessEl: HTMLElement | null = null;
  /** The link the queue has accepted; its details are never kept for a return to this view. */
  let enqueuedResult: ProbeResult | null = null;
  let spotify: SpotifyMatchController | null = null;
  /** The matching panel and what holds it with the confirmed recording's details. */
  let matchView: SpotifyMatchView | null = null;
  let reviewHost: HTMLElement | null = null;
  let resultSlot: HTMLElement | null = null;
  let matchingSessionId = "";

  // ---- Size previews (display only; the download checks free space when it starts) ----
  /** One link: follows the choices, debounced, latest answer only. */
  let previewLine: EstimateLine | null = null;
  const preview = new PreviewEstimator(
    (key) => {
      const [url, options] = JSON.parse(key) as [string, DownloadOptions];
      return api.estimate(url, null, options);
    },
    {
      calls: estimateCalls,
      onChange: () => {
        // A finished estimate is kept for these exact choices, so coming back or choosing them
        // again shows it without asking (preview-session.ts).
        const { status, estimate } = preview.state;
        if (status === "done" && estimate && preview.currentKey) store.previews.remember(preview.currentKey, estimate);
        paintPreviewEstimate();
      },
    },
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
    store.previews.begin(next.url); // a new link replaces the one kept, with its choices and estimates
    mode = "video";
    height = null;
    videoPreset = "compatible";
    audioFormat = "mp3";
    subs = new Set();
    embedSubs = false;
    fastSave = false;
    caption = defaultCaption(captionChoices(next), navigator.languages ?? []);
    transcriptFormat = "txt";
    refreshNotices = [];
    selected = new Set(next.entries.map((_, i) => i));
    phase = "result";
    preview.cancel();
    listRun.reset();
  }

  /** Drops the matching session and its panel; what a return would restore is left alone. */
  function clearSpotifyView() {
    spotify?.dispose();
    spotify = null;
    matchView?.dispose();
    matchView = null;
    reviewHost = null;
    resultSlot = null;
  }

  function clearSpotify() {
    clearSpotifyView();
    store.spotifyMatch = null;
  }

  function createSpotify() {
    matchingSessionId = crypto.randomUUID();
    const controller = new SpotifyMatchController({
      resolve: api.resolveSpotify,
      search: api.searchSpotify,
      probe: api.probe,
    }, (state) => {
      if (spotify !== controller) return;
      ticket += 1;
      const before = phase;
      if (state.stage === "confirmed" && state.confirmed) {
        if (result?.url !== state.confirmed.url) showResult(controller.snapshot().confirmed!);
        mode = "audio";
        freshness = "fresh";
        staleReason = "";
        phase = "result";
      } else {
        result = null;
        phase = "spotify";
        preview.cancel();
        store.previews.clear();
      }
      // While the user types or the results change, only the panel itself is updated in
      // place: rebuilding the stage would drop the query field and an IME composition.
      if (phase !== before || phase === "result" || !matchView || !stage.contains(matchView.el)) render();
      else paintMatching();
    });
    spotify = controller;
    matchView = mountSpotifyMatch({
      setQuery: (query) => { if (!enqueuing) controller.setQuery(query); },
      search: () => { if (!enqueuing) void controller.search(); },
      select: (id) => { if (!enqueuing) controller.select(id); },
      confirm: () => { if (!enqueuing) void controller.confirm(); },
      retry: () => { if (!enqueuing) void controller.open(lastUrl); },
    });
    resultSlot = h("div", { class: "spotify-result" });
    reviewHost = h("div", { class: "spotify-review" }, matchView.el, resultSlot);
    return controller;
  }

  /** Brings the panel up to date; its controls stay frozen while a download is being added. */
  function paintMatching() {
    if (!spotify || !matchView) return;
    matchView.update(spotify.state);
    if (enqueuing) matchView.el.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input, button").forEach((el) => { el.disabled = true; });
  }

  /** Makes the stage hold exactly these nodes, without moving (and so unfocusing) any that stay. */
  function syncStage(wanted: (HTMLElement | null)[]) {
    const nodes = wanted.filter((node): node is HTMLElement => node !== null);
    for (const child of Array.from(stage.children)) if (!nodes.includes(child as HTMLElement)) stage.removeChild(child);
    nodes.forEach((node, index) => {
      if (stage.children[index] !== node) stage.insertBefore(node, stage.children[index] ?? null);
    });
  }

  /** Takes refreshed details while keeping the choices already made. */
  function adoptRefresh(next: ProbeResult) {
    const before = result;
    result = next;
    store.previews.rekey(next.url); // the session is the link's, wherever the lookup settled
    // A subtitle or caption stays chosen only while the refresh still offers that same track
    // (code and kind); otherwise it is dropped and the notice says so. Nothing else changes.
    const kept = refreshTrackChoices({ mode, subs, caption }, before, next, navigator.languages ?? []);
    ({ mode, subs, caption } = kept.choices);
    refreshNotices = kept.notices;
    // A size the refresh no longer offers (or a cap read from heights saved before exact
    // sizes, which for portrait video were the longer side) goes back to Best, so the cap
    // sent is always the chip shown as chosen.
    height = keptQuality(height, qualityChoices(next, next.kind === "playlist"));
    if (!before || before.entries.length !== next.entries.length) selected = new Set(next.entries.map((_, i) => i));
    // Sizes estimated for other videos, or read before the sizes offered changed, do not describe
    // these: the kept estimates go (a new generation), and a playlist's run is forgotten.
    if (!estimatesStillApply(before, next)) {
      store.previews.refreshed();
      listRun.reset();
    }
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
    activeEnqueue = null;
    enqueuing = false;
    clearSpotify();
    skipped = issues;
    if (isSpotifyInput(url)) {
      lastUrl = url;
      result = null;
      freshness = "fresh";
      staleReason = "";
      store.previews.clear();
      phase = "spotify";
      const controller = createSpotify();
      await controller.open(url);
      return;
    }
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
        const changed = refreshSignature(result) !== refreshSignature(probed);
        freshness = "fresh";
        if (changed) {
          adoptRefresh(probed);
          render();
        } else {
          result = probed;
          store.previews.rekey(probed.url);
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
    activeEnqueue = null;
    enqueuing = false;
    clearSpotify();
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
    const transcript = mode === "transcript" ? transcriptOptions(caption, transcriptFormat) : null;
    if (transcript) return transcript;
    const media = mode === "audio" ? "audio" : "video";
    const chosenSubs = [...subs];
    const hasAuto = result?.subtitles.some((s) => s.auto && subs.has(s.code)) ?? false;
    return {
      mode: media,
      maxHeight: media === "video" ? height : null,
      videoPreset: media === "video" ? videoPreset : "compatible",
      audioFormat,
      subtitleLangs: media === "video" ? chosenSubs : [],
      autoSubs: hasAuto,
      embedSubs,
      fastSave,
      transcriptFormat: "txt",
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
    if (mode === "transcript") {
      // Only captions are fetched: there is no media size to estimate.
      preview.cancel();
      paintPreviewEstimate();
      return;
    }
    if (!result || result.kind !== "video" || freshness === "checking") return;
    // The settings the backend reads are part of the key, so a change of them asks again, and so
    // is the refresh generation, so details that changed are never answered from an old estimate.
    const key = JSON.stringify([result.url, currentOptions(), ...estimateSettings(), store.previews.generation]);
    const kept = store.previews.recall(key);
    if (kept && preview.currentKey !== key) preview.restore(key, kept);
    else preview.request(key);
  }

  function paintPreviewEstimate() {
    if (!previewLine) return;
    if (mode === "transcript") return paintLine(previewLine, TEXT_ONLY, "");
    const { size, detail } = describeSingle(preview.state);
    paintLine(previewLine, size, detail, preview.state.status === "loading", false, preview.failed);
  }

  /** The line under a list, and its button, for a run of estimates. */
  function paintRun(run: EstimateRun<EstimateItem>, line: EstimateLine, btn: HTMLButtonElement, count: number) {
    const s = run.state;
    const finished = run.key !== null && !s.running && s.done > 0;
    paintLine(line, describeRun(run), finished ? (aggregate(s.results).tempText ?? "") : "", s.running, true, run.canRetry);
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
    const match = spotify?.state;
    if (spotify && (match?.stage !== "confirmed" || match.confirmed?.url !== result.url || result.audioAvailable !== true)) return;
    enqueuing = true;
    const enqueueToken = Symbol();
    activeEnqueue = enqueueToken;
    const mine = ticket;
    const sent = result;
    const sentSessionId = spotify ? matchingSessionId : null;
    const settleMatch = (accepted: boolean) => {
      if (!sentSessionId) return;
      if (store.spotifyMatch?.sessionId === sentSessionId) {
        if (accepted) store.spotifyMatch = null;
        else store.spotifyMatch.enqueuePending = false;
      }
      store.emit("spotifyMatch", { sessionId: sentSessionId, accepted });
    };
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
      store.previews.adding(sent.url); // a view left from here is not shown as new on a return
      const origin = match?.track ? { kind: "spotifyYouTube" as const, spotifyUrl: match.track.url, spotifyTitle: match.track.title, spotifyArtists: [...match.track.artists] } : null;
      const added = await api.enqueue(items.map((item) => ({ ...item, matchOrigin: origin })), currentOptions(), spotify ? sent.url : lastUrl || sent.url);
      // Usually already applied from the jobs-added event; the revisions make this a no-op.
      store.addJobs(added);
      enqueuedResult = sent;
      // The mounted view handles its own success below; a replacement view receives this.
      if (mine !== ticket) settleMatch(true);
      store.previews.clearFor(sent.url); // added to the queue: its details and estimates are done with
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
      clearSpotify();
      input.value = "";
      fitInput();
      lastUrl = "";
      result = null;
      // Back to the list this link was looked up over, if there is one.
      phase = store.batch ? "batch" : "idle";
      render();
    } catch (error) {
      settleMatch(false);
      store.previews.addFailed(sent.url);
      toast(errorMessage(error), { tone: "error" });
      if (trigger) trigger.disabled = false;
    } finally {
      if (activeEnqueue === enqueueToken) {
        enqueuing = false;
        activeEnqueue = null;
        if (spotify) render();
      }
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

  /** The user has chosen again, so what a refresh said about their old choices no longer applies. */
  function dismissRefreshNotices() {
    if (refreshNotices.length === 0) return;
    refreshNotices = [];
    paintFreshness();
  }

  function paintFreshness() {
    if (!freshnessEl || !result) return;
    const r = result;
    const same = store.history.filter((item) => item.url === r.url || item.url === lastUrl);
    // A saved transcript is not a downloaded video: say which it was, preferring the media.
    const previous = same.find((item) => item.mode !== "transcript") ?? same[0];
    const parts = [
      previous ? `${previous.mode === "transcript" ? "Transcript saved" : "Downloaded"} ${formatDate(previous.finishedAt)}` : null,
      freshness === "checking" ? "Checking for changes…" : null,
      freshness === "stale" ? (staleReason === CHECK_INTERRUPTED ? staleReason : `Couldn't check for changes: ${staleReason}`) : null,
      ...refreshNotices,
    ].filter(Boolean) as string[];
    freshnessEl.textContent = parts.join("  ·  ");
    freshnessEl.hidden = parts.length === 0;
    freshnessEl.dataset.tone = freshness === "stale" || refreshNotices.length > 0 ? "warn" : "";
  }

  function renderEntries(r: ProbeResult): HTMLElement {
    const countLabel = h("span", { class: "entries-count" });
    const list = h("div", { class: "entries", attrs: { role: "group", "aria-label": "Videos in this playlist" } });
    // Created with a label: an icon-only button is a fixed 28 px wide.
    const downloadBtn: HTMLButtonElement = button({ label: "Download", icon: "download", variant: "primary", onClick: () => void startDownload(downloadBtn) });
    const listLine = estimateLine(() => void listRun.retryFailed(listKey()), "Retry Failed", "Asks again only for the videos whose size could not be estimated. Sizes already read stay. Download is not affected.");
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
    const qualities = qualityChoices(r, playlist);
    const captions = captionChoices(r);

    // The Type control is built once and stays while the rows below it change, so its
    // thumb slides to the new choice (a CSS spring; still with reduced motion) and the
    // keyboard stays on it. Transcripts are made one video at a time.
    const typeHint = h("span");
    const typeSeg = segmented<DownloadMode>(
      "Type",
      [
        { value: "video", label: "Video", icon: "video" },
        { value: "audio", label: "Audio Only", icon: "music" },
        ...(playlist
          ? []
          : [
              {
                value: "transcript" as const,
                label: "Transcript",
                icon: "captions" as const,
                disabled: !captions.available,
                title: captions.available ? "Text from the site's captions" : NO_CAPTIONS,
              },
            ]),
      ],
      mode,
      (next) => {
        mode = next;
        dismissRefreshNotices();
        paint();
      },
    );
    const typeRow = row("Type", typeSeg.el, typeHint);
    typeRow.hidden = spotify !== null;
    const panel = group(typeRow);
    const body = h("div", { class: "options-body" }, panel);

    const paint = () => {
      const rows: (Node | null)[] = [];
      const hint = playlist
        ? "Transcripts are made one video at a time: open a video's own link to save its captions."
        : captions.available
          ? ""
          : `${NO_CAPTIONS}, so there is no transcript to save.`;
      typeHint.textContent = hint;
      typeHint.hidden = !hint;

      if (mode === "transcript") {
        const formatHint = h("span", { text: TRANSCRIPT_FORMATS.find((f) => f.value === transcriptFormat)?.hint ?? "" });
        rows.push(
          row(
            "Captions",
            captionSelect(captions, caption, (key) => {
              caption = key;
              dismissRefreshNotices();
            }),
            "Made from the site's own captions; Claws does not recognise speech. Automatic captions are generated by the site and can contain errors.",
          ),
          row(
            "Save as",
            chipRadio<TranscriptFormat>("Transcript format", TRANSCRIPT_FORMATS, transcriptFormat, (value) => {
              transcriptFormat = value;
              formatHint.textContent = TRANSCRIPT_FORMATS.find((f) => f.value === value)?.hint ?? "";
            }),
            formatHint,
          ),
        );
      } else if (mode === "video") {
        rows.push(
          row(
            "Quality",
            qualityChips(qualities, height, (value) => {
              height = value;
              scheduleEstimate();
            }),
            qualities.length === 0
              ? "Quality options appear once the site reports them."
              : playlist
                ? "Each video is saved at the largest size it has, up to this limit. Nothing is upscaled."
                : hasExactSizes(r)
                  ? "The sizes this video is offered in, by the shorter side. Nothing is upscaled."
                  : "Saved at the largest size this video has, up to this limit on its shorter side. Nothing is upscaled.",
          ),
          presetRow(videoPreset, (next) => {
            videoPreset = next;
            scheduleEstimate();
          }),
        );
        if (r.subtitles.length > 0) {
          rows.push(row("Subtitles", subtitleChips(r.subtitles, subs, () => {
            dismissRefreshNotices();
            paint();
          }), subtitleLimitHint(subs.size)));
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
            spotify ? `${audioHint()} FLAC and WAV conversion cannot improve the YouTube source quality.` : audioHint(),
          ),
        );
      }
      if (mode !== "transcript") rows.push(fastSaveRow());
      // Everything after the Type row is replaced; the Type row itself stays.
      while (typeRow.nextSibling) typeRow.nextSibling.remove();
      append(panel, rows);
      paintActionLabel?.();
      scheduleEstimate();
    };

    // Fast Save names exactly what it changes for these choices (processing-options.ts, the
    // mirror of the Rust ProcessingPolicy); it never changes the video or audio downloaded.
    const fastSaveRow = () => {
      const hint = fastSaveHint({
        mode: mode === "audio" ? "audio" : "video",
        audioFormat,
        metadata: store.settings.embedMetadata,
        embeddedSubs: mode === "video" && subs.size > 0 && embedSubs,
        fastSave,
      });
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
      previewLine = estimateLine(() => preview.retry());
      paintPreviewEstimate();
      wrap.append(
        previewLine.root,
        h(
          "div",
          { class: "actions" },
          h("span", { class: "save-to", text: `Saves to ${folderName(store.settings.outputDir)}` }),
          (() => {
            const btn: HTMLButtonElement = button({ label: "Download", icon: "download", variant: "primary", onClick: () => void startDownload(btn) });
            btn.disabled = enqueuing;
            paintActionLabel = () => {
              // The "Added" confirmation stays as it is until the view resets.
              if (btn.classList.contains("confirmed")) return;
              const transcript = mode === "transcript";
              replace(btn, icon(transcript ? "captions" : "download", 16), h("span", { text: transcript ? "Save Transcript" : "Download" }));
            };
            paintActionLabel();
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
    if (summary.added > 0 && store.batch === session) session.estimate = null; // sized links are in the queue now
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
    const qualityRow = row(
      "Quality",
      qualityChips(capChoices(), choices.maxHeight, (value) => {
        choices.maxHeight = value;
        scheduleCounts(); // an estimate for other choices no longer applies
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
          clearSpotify();
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
    paintActionLabel = null;
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
        if (spotify && reviewHost && resultSlot) {
          replace(resultSlot, shown);
          syncStage([back, issues, reviewHost]);
          paintMatching();
        } else replace(stage, back, issues, shown);
        if (changed) stageAnimation = settleIn(shown);
        break;
      }
      case "spotify":
        if (resultSlot) clear(resultSlot);
        syncStage([back, issues, reviewHost]);
        paintMatching();
        break;
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
    if (looksLikeUrl(pasted) || isSpotifyInput(pasted)) void submit();
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
  const offSpotify = store.on("spotifyMatch", (detail) => {
    const outcome = detail as { sessionId: string; accepted: boolean };
    if (!spotify || matchingSessionId !== outcome.sessionId) return;
    enqueuing = false;
    if (outcome.accepted) {
      clearSpotify();
      result = null;
      lastUrl = "";
      input.value = "";
      phase = store.batch ? "batch" : "idle";
    }
    render();
  });

  // Back from another view: the link's details, the choices made over them and the estimates
  // already read come back as they were left. Nothing that was still running is revived.
  const matchingReturn = store.spotifyMatch;
  const returning = matchingReturn ? null : store.previews.take();
  if (matchingReturn) {
    lastUrl = matchingReturn.snapshot.url;
    input.value = lastUrl;
    fitInput();
    createSpotify().restore(matchingReturn.snapshot);
    matchingSessionId = matchingReturn.sessionId;
    enqueuing = matchingReturn.enqueuePending;
    audioFormat = matchingReturn.audioFormat;
    fastSave = matchingReturn.fastSave;
  }
  if (returning) {
    result = returning.result;
    lastUrl = returning.typedUrl;
    input.value = returning.typedUrl; // the link field shows the link the details are for
    fitInput();
    ({ mode, height, videoPreset, audioFormat, embedSubs, fastSave, caption, transcriptFormat } = returning.choices);
    subs = new Set(returning.choices.subs);
    selected = new Set(returning.selected);
    freshness = returning.freshness;
    staleReason = returning.staleReason;
    refreshNotices = returning.notices;
    if (returning.list) listRun.restore(returning.list);
    phase = "result";
  }
  // The batch's own sizes, only when its list is what opens (a link's details drawn over a kept
  // batch forget its run, as they always did).
  if (phase === "batch" && store.batch?.estimate) {
    batchRun.restore(store.batch.estimate); // checked against the rows and choices when it paints
    store.batch.estimate = null;
  }

  /** Leaving the view: keeps what a return needs, as plain data, and stops what is running. */
  function saveSession() {
    if (spotify) {
      store.spotifyMatch = enqueuedResult !== null && result === enqueuedResult ? null : { snapshot: spotify.snapshot(), audioFormat, fastSave, sessionId: matchingSessionId, enqueuePending: enqueuing };
      store.previews.clear();
      return;
    }
    if (phase === "result" && result && result !== enqueuedResult) {
      listRun.cancel(); // what it read stays; the calls still on their way are ignored
      const checking = freshness === "checking";
      store.previews.save({
        url: result.url,
        typedUrl: lastUrl || result.url,
        result,
        choices: { mode, height, videoPreset, audioFormat, subs: [...subs], embedSubs, fastSave, caption, transcriptFormat },
        selected: [...selected].sort((a, b) => a - b),
        // The check for changes is dropped with the view, so it cannot be shown as still running.
        freshness: freshness === "checking" ? "stale" : freshness,
        staleReason: checking ? CHECK_INTERRUPTED : staleReason,
        notices: refreshNotices,
        list: listRun.snapshot(),
      });
    } else {
      store.previews.clear(); // a loading or failed lookup, a batch or an added link: nothing to come back to
    }
    if (store.batch && phase === "batch") {
      batchRun.cancel();
      store.batch.estimate = batchRun.snapshot();
    }
  }

  render();
  input.focus();
  takePaste();

  return () => {
    saveSession();
    clearSpotifyView();
    offPaste();
    offFocus();
    offTools.forEach((off) => off());
    offHistory();
    offBatch();
    offBatchRow();
    offSpotify();
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
