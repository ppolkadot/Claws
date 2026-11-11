// Pieces the Download page's parts share: the choices made for one link, the small
// controls, and the size-estimate line. Nothing here keeps state of its own except the
// limit on estimate calls, which is shared on purpose.
import type { DownloadMode, DownloadOptions, ProbeResult, SponsorBlockMode, TranscriptFormat, VideoPreset } from "../api";
import { CallLimit, EstimateRun, aggregate, describeRun } from "../download-estimates";
import { parseLinks, type BatchRow, type ParsedInput } from "../download-input";
import { formatDuration } from "../format";
import { VIDEO_PRESETS } from "../resolution";
import { captionKey, type CaptionChoices } from "../transcript-options";
import { store, type BatchChoices, type EstimateItem } from "../store";
import { button, chipRadio, segmented } from "../ui/components";
import { h } from "../ui/dom";
import { icon } from "../ui/icons";
import { row } from "../ui/layout";

/** What the user has chosen for the link on screen. The parts edit it in place. */
export interface LinkChoices {
  mode: DownloadMode;
  height: number | null;
  videoPreset: VideoPreset;
  audioFormat: string;
  subs: Set<string>;
  embedSubs: boolean;
  fastSave: boolean;
  caption: string | null;
  transcriptFormat: TranscriptFormat;
  /** Ticked entries of a playlist, by position. */
  selected: Set<number>;
  sponsorblock: SponsorBlockMode;
  /** The kinds of segment SponsorBlock applies to; never empty. */
  sponsorCategories: Set<string>;
  /** A playlist's videos go into a folder named after it, numbered by position. */
  playlistFolder: boolean;
}

/** SponsorBlock's kinds of segment, by yt-dlp's names (download_policy.rs has the same list). */
export const SPONSOR_CATEGORIES = [
  { value: "sponsor", label: "Sponsor" },
  { value: "selfpromo", label: "Self-promotion" },
  { value: "interaction", label: "Interaction reminder" },
  { value: "intro", label: "Intro" },
  { value: "outro", label: "Outro" },
  { value: "preview", label: "Preview" },
  { value: "music_offtopic", label: "Non-music section" },
  { value: "filler", label: "Filler" },
];

/** SponsorBlock only knows YouTube videos (yt-dlp applies it to no other site). */
export function offersSponsorBlock(result: { site: string | null }): boolean {
  return (result.site ?? "").toLowerCase().startsWith("youtube");
}

/** Batch rows built per frame, as in the Queue view: about a frame's worth of work. */
export const ROWS_PER_FRAME = 40;
export const ISSUES_SHOWN = 6;

export const AUDIO_FORMATS = [
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
export function presetRow(value: VideoPreset, onChange: (next: VideoPreset) => void): HTMLElement {
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

export const NO_CAPTIONS = "No captions available for this video";
export const TEXT_ONLY = "Text only — the video is not downloaded";

export function captionSelect(choices: CaptionChoices, value: string | null, onChange: (key: string) => void): HTMLSelectElement {
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

export function qualityChips(choices: { value: number; label: string; hint: string }[], value: number | null, onChange: (next: number | null) => void): HTMLElement {
  return chipRadio<number>("Quality", [{ value: 0, label: "Best" }, ...choices], value ?? 0, (next) => onChange(next === 0 ? null : next));
}

export function looksLikeUrl(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
}

/** Text with two or more links on separate lines: filled in, never looked up by itself. */
export function looksLikeLinkList(text: string): boolean {
  return /[\r\n]/.test(text.trim()) && parseLinks(text).links.length >= 2;
}

export function audioHint(): string {
  return store.settings.audioSelection === "preferMatching"
    ? "Original keeps the site's own audio. For other formats, a stream already in that format is saved without conversion when the site has one."
    : "Original keeps the site's own audio without re-encoding. Other formats are converted from the site's best audio.";
}

export const shorten = (text: string, max = 70) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

export function batchOptions(choices: BatchChoices): DownloadOptions {
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

export type EstimateLine = {
  root: HTMLElement;
  text: HTMLElement;
  size: HTMLElement;
  detail: HTMLElement;
  /** Visually hidden; written only when an estimate starts or settles, never per step. */
  status: HTMLElement;
  retry: HTMLButtonElement;
  busy: boolean;
};

/**
 * The size and free-space lines of an estimate, separate so neither hides the other, and the
 * button that asks again for an estimate that failed (`onRetry`; the estimate state itself
 * enforces one retry at a time through the shared call limit).
 */
export function estimateLine(onRetry: () => void, retryLabel = "Retry", retryTitle = "Asks again for this size. Download is not affected."): EstimateLine {
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

export function setText(el: HTMLElement, text: string) {
  if (el.textContent !== text) el.textContent = text;
}

/**
 * Paints an estimate. The free-space line keeps its height while it is empty, so the
 * Download button below does not move each time an estimate starts and settles. A screen
 * reader hears the outcome once it settles (and, with `announceStart`, that it started).
 */
export function paintLine(line: EstimateLine, size: string, detail: string, busy = false, announceStart = false, canRetry = false) {
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
export const STOP_GUARD_MS = 600;

/**
 * The Estimate Sizes button of a list. It becomes Stop Estimating while a run goes; the
 * second click of a double click, or Enter held down, does not stop the run it started.
 */
export function estimateButton(run: EstimateRun<EstimateItem>, start: () => void): HTMLButtonElement {
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
export function runKey(options: DownloadOptions, settings: unknown[]): string {
  return JSON.stringify([options, settings]);
}

/**
 * Estimate calls running at once, across the single preview, a playlist's run and the
 * batch's run, and across views: a reset run's calls keep their slots until they answer.
 */
export const estimateCalls = new CallLimit();

export interface BatchRowRefs {
  pick: HTMLButtonElement;
  thumb: HTMLElement;
  thumbFor: string | null;
  title: HTMLElement;
  sub: HTMLElement;
  side: HTMLElement;
  sideFor: string;
}

export function paintRun(run: EstimateRun<EstimateItem>, line: EstimateLine, btn: HTMLButtonElement, count: number) {
  const s = run.state;
  const finished = run.key !== null && !s.running && s.done > 0;
  paintLine(line, describeRun(run), finished ? (aggregate(s.results).tempText ?? "") : "", s.running, true, run.canRetry);
  setText(btn.querySelector("span")!, s.running ? "Stop Estimating" : "Estimate Sizes");
  const title = s.running
    ? "Stops estimating. Sizes already read stay; estimates already running finish in the background and are ignored."
    : `Reads the size of each selected video, up to ${run.maxItems}, three at a time. Nothing is read until you ask.`;
  if (btn.title !== title) btn.title = title;
  btn.setAttribute("aria-disabled", String(!s.running && count === 0));
}

/** The settings the backend reads for an estimate; a change of them asks again. */
export const estimateSettings = () => [store.settings.audioSelection, store.settings.embedMetadata];

export function thumbnailEl(url: string | null, cls: string): HTMLElement {
  if (!url) return h("div", { class: `${cls} thumb-fallback` }, icon("video", 22));
  const img = h("img", { class: cls, attrs: { src: url, alt: "", loading: "lazy", decoding: "async", referrerpolicy: "no-referrer", draggable: false } });
  img.addEventListener("error", () => img.replaceWith(h("div", { class: `${cls} thumb-fallback` }, icon("video", 22))));
  return img;
}

export function rowNote(r: BatchRow<ProbeResult>): [string, string] {
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

export function renderIssues(parsed: ParsedInput): HTMLElement | null {
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
