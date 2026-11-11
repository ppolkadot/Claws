import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type DownloadOptions, type ProbeResult } from "../api";
import { EstimateRun, PreviewEstimator, describeSingle } from "../download-estimates";
import { MAX_LINKS, isBatchInput, itemKey, parseLinks, type BatchRow, type ParsedInput } from "../download-input";
import { formatDate, formatDuration, folderName } from "../format";
import { applyLastChoices, loadLastChoices, saveLastChoices } from "../last-choices";
import { keptQuality, qualityChoices } from "../resolution";
import { captionChoices, defaultCaption, refreshSignature, refreshTrackChoices, transcriptOptions } from "../transcript-options";
import { CHECK_INTERRUPTED, estimatesStillApply } from "../preview-session";
import { store, type EstimateItem } from "../store";
import { button, toast } from "../ui/components";
import { clear, h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { group, sectionTitle } from "../ui/layout";
import { settleIn } from "../ui/motion";
import { mountBatch, type BatchView } from "./download-batch";
import { mountOptions } from "./download-options";
import { mountEntries } from "./download-playlist";
import {
  AUDIO_FORMATS,
  TEXT_ONLY,
  estimateCalls,
  estimateLine,
  estimateSettings,
  looksLikeUrl,
  paintLine,
  renderIssues,
  thumbnailEl,
  type EstimateLine,
  type LinkChoices,
} from "./download-shared";
import { mountSetupPanel } from "./setup-panel";
import type { MountView } from "./shell";
import { SpotifyMatchController, isSpotifyInput } from "../spotify-match";
import { mountSpotifyMatch, type SpotifyMatchView } from "./spotify-match";

export { looksLikeLinkList, looksLikeUrl } from "./download-shared";

type Phase = "idle" | "loading" | "result" | "error" | "batch" | "spotify";


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
  let skipped: ParsedInput | null = null;
  /** Bumped by every submission; an older lookup's reply is dropped. */
  let ticket = 0;
  let stageAnimation: Animation | null = null;
  let batchView: BatchView | null = null;

  /** The choices for the link on screen; the option and playlist parts edit them in place. */
  const choice: LinkChoices = {
    mode: "video",
    height: null,
    videoPreset: "compatible",
    audioFormat: "mp3",
    subs: new Set<string>(),
    embedSubs: false,
    fastSave: false,
    caption: null,
    transcriptFormat: "txt",
    selected: new Set<number>(),
  };
  let paintActionLabel: (() => void) | null = null;
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
  let matchView: SpotifyMatchView | null = null;
  let reviewHost: HTMLElement | null = null;
  let resultSlot: HTMLElement | null = null;
  let matchingSessionId = "";

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
  let paintListEstimate: (() => void) | null = null;
  const listRun = new EstimateRun<EstimateItem>(estimateItem, { ...runOptions, onChange: () => paintListEstimate?.() });
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

  // Nothing is installed until the panel's button is pressed.
  const offSetup = mountSetupPanel(setup);

  function showResult(next: ProbeResult) {
    result = next;
    store.previews.begin(next.url); // a new link replaces the one kept, with its choices and estimates
    choice.mode = "video";
    choice.height = null;
    choice.videoPreset = "compatible";
    choice.audioFormat = "mp3";
    choice.subs = new Set();
    choice.embedSubs = false;
    choice.fastSave = false;
    choice.caption = defaultCaption(captionChoices(next), navigator.languages ?? []);
    choice.transcriptFormat = "txt";
    // The choices of the last download come back, as far as this link offers them.
    const last = loadLastChoices();
    if (last) {
      const playlist = next.kind === "playlist";
      const kept = applyLastChoices(last, {
        heights: qualityChoices(next, playlist).map((c) => c.value),
        audioFormats: AUDIO_FORMATS.map((f) => f.value),
        transcript: !playlist && captionChoices(next).available,
      });
      choice.mode = kept.mode;
      choice.audioFormat = kept.audioFormat;
      choice.height = kept.height;
      choice.videoPreset = kept.videoPreset;
      choice.fastSave = kept.fastSave;
    }
    refreshNotices = [];
    choice.selected = new Set(next.entries.map((_, i) => i));
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
      const focusDownload = state.stage === "confirmed"
        && document.activeElement === matchView?.el.querySelector(".spotify-status");
      if (state.stage === "confirmed" && state.confirmed) {
        if (result?.url !== state.confirmed.url) showResult(controller.snapshot().confirmed!);
        choice.mode = "audio";
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
      if (focusDownload) resultSlot?.querySelector<HTMLButtonElement>(".actions .btn-primary")?.focus({ preventScroll: true });
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

  function paintMatching() {
    if (!spotify || !matchView) return;
    reviewHost?.setAttribute("data-confirmed", String(spotify.state.stage === "confirmed"));
    if (resultSlot) resultSlot.hidden = spotify.state.stage !== "confirmed";
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

  function adoptRefresh(next: ProbeResult) {
    const before = result;
    result = next;
    store.previews.rekey(next.url); // the session is the link's, wherever the lookup settled
    // A subtitle or caption stays chosen only while the refresh still offers that same track
    // (code and kind); otherwise it is dropped and the notice says so. Nothing else changes.
    const kept = refreshTrackChoices({ mode: choice.mode, subs: choice.subs, caption: choice.caption }, before, next, navigator.languages ?? []);
    Object.assign(choice, kept.choices);
    refreshNotices = kept.notices;
    // A size the refresh no longer offers (or a cap read from heights saved before exact
    // sizes, which for portrait video were the longer side) goes back to Best, so the cap
    // sent is always the chip shown as chosen.
    choice.height = keptQuality(choice.height, qualityChoices(next, next.kind === "playlist"));
    if (!before || before.entries.length !== next.entries.length) choice.selected = new Set(next.entries.map((_, i) => i));
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
      return showError("Couldn't read that link", "yt-dlp is not installed yet. Press Set Up Everything above, then try again.");
    }
    lastUrl = url;

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
    if (!store.tools.ytdlp) return showError("Couldn't read those links", "yt-dlp is not installed yet. Press Set Up Everything above, then try again.");
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
    const transcript = choice.mode === "transcript" ? transcriptOptions(choice.caption, choice.transcriptFormat) : null;
    if (transcript) return transcript;
    const media = choice.mode === "audio" ? "audio" : "video";
    const chosenSubs = [...choice.subs];
    const hasAuto = result?.subtitles.some((s) => s.auto && choice.subs.has(s.code)) ?? false;
    return {
      mode: media,
      maxHeight: media === "video" ? choice.height : null,
      videoPreset: media === "video" ? choice.videoPreset : "compatible",
      audioFormat: choice.audioFormat,
      subtitleLangs: media === "video" ? chosenSubs : [],
      autoSubs: hasAuto,
      embedSubs: choice.embedSubs,
      fastSave: choice.fastSave,
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
    if (choice.mode === "transcript") {
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
    if (choice.mode === "transcript") return paintLine(previewLine, TEXT_ONLY, "");
    const { size, detail } = describeSingle(preview.state);
    paintLine(previewLine, size, detail, preview.state.status === "loading", false, preview.failed);
  }


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
            .filter((_, i) => choice.selected.has(i))
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
      store.previews.adding(sent.url); // a view left from here is not shown as new on a return
      const origin = match?.track ? { kind: "spotifyYouTube" as const, spotifyUrl: match.track.url, spotifyTitle: match.track.title, spotifyArtists: [...match.track.artists] } : null;
      const added = await api.enqueue(items.map((item) => ({ ...item, matchOrigin: origin })), currentOptions(), spotify ? sent.url : lastUrl || sent.url);
      // Usually already applied from the jobs-added event; the revisions make this a no-op.
      store.addJobs(added);
      // Remembered for the next link. A Spotify match is always audio, so it says nothing
      // about what the user would choose.
      if (!spotify) {
        saveLastChoices({
          mode: choice.mode,
          audioFormat: choice.audioFormat,
          height: choice.height,
          videoPreset: choice.videoPreset === "original" ? "original" : "compatible",
          fastSave: choice.fastSave,
        });
      }
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

  function renderResult(r: ProbeResult): HTMLElement {
    const wrap = h("div", { class: "result" }, renderPreview(r));
    if (r.kind === "playlist" && r.entries.length === 0) {
      wrap.append(h("div", { class: "notice" }, icon("alert", 16), h("span", { text: "This playlist has no downloadable videos." })));
      return wrap;
    }
    wrap.append(
      sectionTitle("Options"),
      mountOptions(r, choice, {
        spotify: spotify !== null,
        dismissRefreshNotices,
        scheduleEstimate,
        paintActionLabel: () => paintActionLabel?.(),
      }),
    );
    if (r.kind === "playlist") {
      const entries = mountEntries(r, choice, { run: listRun, currentOptions, startDownload: (trigger) => void startDownload(trigger) });
      paintListEstimate = entries.paintEstimate;
      wrap.append(entries.el);
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
              const transcript = choice.mode === "transcript";
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
    const issues = skipped && phase !== "idle" && phase !== "batch" ? renderIssues(skipped) : null;
    switch (phase) {
      case "loading":
        // The preview card's own shape, so the page does not move when the details arrive.
        replace(
          stage,
          back,
          issues,
          h(
            "div",
            { class: "result" },
            h(
              "section",
              { class: "preview preview-skeleton glass", attrs: { role: "status", "aria-label": "Reading the link" } },
              h("div", { class: "preview-thumb" }),
              h("div", { class: "preview-info" }, h("div", { class: "skeleton-line" }), h("div", { class: "skeleton-line" }), h("div", { class: "skeleton-line" })),
              h("span", { class: "visually-hidden", text: "Reading the link…" }),
            ),
          ),
        );
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
        batchView = mountBatch(store.batch, { run: batchRun, input, stage, refreshCurrent: () => batchView?.refresh() });
        paintBatchEstimate = batchView.paintEstimate;
        replace(stage, batchView.el);
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
    choice.audioFormat = matchingReturn.audioFormat;
    choice.fastSave = matchingReturn.fastSave;
  }
  if (returning) {
    result = returning.result;
    lastUrl = returning.typedUrl;
    input.value = returning.typedUrl; // the link field shows the link the details are for
    fitInput();
    const kept = returning.choices;
    choice.mode = kept.mode;
    choice.height = kept.height;
    choice.videoPreset = kept.videoPreset;
    choice.audioFormat = kept.audioFormat;
    choice.embedSubs = kept.embedSubs;
    choice.fastSave = kept.fastSave;
    choice.caption = kept.caption;
    choice.transcriptFormat = kept.transcriptFormat;
    choice.subs = new Set(kept.subs);
    choice.selected = new Set(returning.selected);
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
      store.spotifyMatch = enqueuedResult !== null && result === enqueuedResult ? null : { snapshot: spotify.snapshot(), audioFormat: choice.audioFormat, fastSave: choice.fastSave, sessionId: matchingSessionId, enqueuePending: enqueuing };
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
        choices: {
          mode: choice.mode,
          height: choice.height,
          videoPreset: choice.videoPreset,
          audioFormat: choice.audioFormat,
          subs: [...choice.subs],
          embedSubs: choice.embedSubs,
          fastSave: choice.fastSave,
          caption: choice.caption,
          transcriptFormat: choice.transcriptFormat,
        },
        selected: [...choice.selected].sort((a, b) => a - b),
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
    offSetup();
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
