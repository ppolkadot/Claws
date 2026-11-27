import { api, errorMessage } from "../api";
import { formatBytes, folderName } from "../format";
import { store } from "../store";
import { button, confirmSheet, segmented, toast, type SegmentedHandle } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { paintJobStatus } from "../ui/job-status";
import { group, progressBar, row, sectionTitle, setProgress } from "../ui/layout";
import { progressMotion, type ProgressState } from "../ui/progress-motion";
import {
  estimateLine,
  GIF_SECONDS,
  libraryLine,
  longRunMessage,
  missingForStart,
  RECORD_LENGTHS,
  RECORD_MAX_SECONDS,
  recordNotes,
  savedLine,
  stageLine,
  type ExportFormat,
  type WorkshopSession,
} from "../workshop-session";
import { thumbnailEl } from "./download-shared";

export interface WorkshopView {
  el: HTMLElement;
  dispose(): void;
}

/** The Download screen's panel for one Wallpaper Engine item. */
export function mountWorkshop(session: WorkshopSession): WorkshopView {
  const item = session.item;
  const meta = ["Wallpaper Engine", item.wallpaperType, item.fileSize ? formatBytes(item.fileSize) : null].filter(Boolean).join(" · ");
  const preview = h(
    "section",
    { class: "preview glass", attrs: { "aria-label": "Workshop item" } },
    thumbnailEl(item.previewUrl, "preview-thumb"),
    h(
      "div",
      { class: "preview-info" },
      h("h2", { class: "preview-title", text: item.title || `Workshop item ${item.id}` }),
      h("p", { class: "preview-meta", text: meta }),
    ),
  );
  const required = item.requiredItems.length
    ? h(
        "p",
        { class: "notice" },
        h("span", {
          text:
            item.requiredItems.length === 1
              ? "This wallpaper needs 1 other Workshop item. Download it too, or the wallpaper may not work."
              : `This wallpaper needs ${item.requiredItems.length} other Workshop items. Download them too, or the wallpaper may not work.`,
        }),
      )
    : null;

  // Steam library: the only source. Subscribing in Steam is what puts the item there.
  const libraryText = h("span", { class: "row-note" });
  const subscribe = button({
    label: "Subscribe in Steam",
    variant: "secondary",
    size: "small",
    onClick: () => {
      void api.openWorkshopItem(session.url).then(
        () => session.watchLibrary(),
        (error) => toast(errorMessage(error), { tone: "error" }),
      );
    },
  });
  // No Steam: the one line that says what is needed, with the item's page to start from.
  const viewInSteam = button({
    label: "View in Steam",
    variant: "secondary",
    size: "small",
    onClick: () => void api.openWorkshopItem(session.url).catch((error) => toast(errorMessage(error), { tone: "error" })),
  });
  const stopWatching = button({ label: "Stop waiting", variant: "plain", size: "small", onClick: () => session.stopWatching() });
  const libraryRow = row("Steam library", h("div", { class: "inline" }, libraryText, subscribe, viewInSteam, stopWatching));

  // Format. ZIP keeps the project and works for every item. A video project can also be saved
  // as MP4 (its own file when it is H.264, converted otherwise) or as a short GIF. The choices
  // are rebuilt only when they change, so the thumb never jumps.
  const formatHint = h("span", { class: "row-note" });
  // What the choice is expected to take, in caption style, under the choice.
  const estimate = h("span", { class: "workshop-estimate", attrs: { "aria-live": "polite" } });
  const formatControl = h("div", { class: "row-control-slot" });
  let formatKey = "";
  let formatSeg: SegmentedHandle<ExportFormat> | null = null;
  function paintFormats() {
    const choices = session.choices;
    const key = choices.map((choice) => `${choice.value}:${choice.label}`).join("|");
    if (key !== formatKey) {
      formatKey = key;
      formatSeg = segmented<ExportFormat>("Save as", choices, session.format, (next) => session.setFormat(next));
      replace(formatControl, formatSeg.el);
    }
    formatSeg?.set(session.format);
    formatSeg?.el.querySelectorAll("button").forEach((b) => b.setAttribute("aria-disabled", session.running ? "true" : "false"));
  }
  const formatRow = row("Save as", formatControl, h("span", {}, formatHint, estimate));

  // GIF length: the first 30 or 60 seconds. A long loop makes a huge file and takes very long.
  const gifSeg = segmented<string>(
    "GIF length",
    GIF_SECONDS.map((seconds) => ({ value: String(seconds), label: `${seconds} s` })),
    String(session.gifSeconds),
    (next) => session.setGifSeconds(Number(next)),
  );
  const gifRow = row("GIF length", gifSeg.el, "The first seconds of the loop, at most 720p.");

  // Recording a scene or web wallpaper: how long, how large, how smooth. Each choice is made from
  // what this PC can really do (the backend says which sizes and rates), and a size that is not
  // available stays visible with the reason.
  const lengthSeg = segmented<string>(
    "Recording length",
    [...RECORD_LENGTHS.map((seconds) => ({ value: String(seconds), label: `${seconds} s` })), { value: "custom", label: "Custom" }],
    String(session.recordLength),
    (next) => (next === "custom" ? session.setLengthCustom(true) : session.setRecordLength(Number(next), false)),
  );
  const lengthRow = row("Length", lengthSeg.el);
  const customLength = h("input", {
    class: "text-input",
    attrs: { id: "workshop-length", type: "number", min: 1, max: RECORD_MAX_SECONDS, step: 5, inputmode: "numeric", value: String(session.recordLength) },
  });
  customLength.addEventListener("change", () => session.setRecordLength(Number(customLength.value), true));
  const customRow = row("Custom length", h("div", { class: "inline" }, customLength, h("span", { class: "row-note", text: "seconds, up to 10 minutes" })), undefined, "workshop-length");
  const sizeControl = h("div", { class: "row-control-slot" });
  let sizeKey = "";
  let sizeSeg: SegmentedHandle<string> | null = null;
  function paintSizes() {
    const sizes = session.record?.sizes ?? [];
    const key = sizes.map((size) => `${size.height}:${size.available}`).join("|");
    if (key !== sizeKey) {
      sizeKey = key;
      sizeSeg = segmented<string>(
        "Size",
        sizes.map((size) => ({ value: String(size.height), label: size.label, disabled: !size.available, title: size.reason ?? `${size.width}×${size.height}` })),
        String(session.size?.height ?? ""),
        (next) => session.setRecordHeight(Number(next)),
      );
      replace(sizeControl, sizeSeg.el);
    }
    sizeSeg?.set(String(session.size?.height ?? ""));
    sizeSeg?.el.querySelectorAll("button").forEach((b) => b.setAttribute("aria-disabled", session.running || b.getAttribute("aria-disabled") === "true" ? "true" : "false"));
  }
  const sizeRow = row("Size", sizeControl, h("span", { class: "row-note", attrs: { id: "workshop-size-note" } }));
  const fpsSeg = segmented<string>(
    "Frame rate",
    [
      { value: "30", label: "30 fps" },
      { value: "60", label: "60 fps" },
    ],
    "30",
    (next) => session.setRecordFps(Number(next)),
  );
  const fpsRow = row("Frame rate", fpsSeg.el);
  const recordNote = h("div", { class: "workshop-notes", attrs: { role: "note" } });

  const options = group(libraryRow, formatRow, gifRow, lengthRow, customRow, sizeRow, fpsRow);

  // Actions and status
  const reason = h("span", { class: "row-hint workshop-reason" });
  const saveTo = h("span", { class: "save-to" });
  const save: HTMLButtonElement = button({ label: "Save ZIP", variant: "primary", onClick: () => void start() });
  const cancel: HTMLButtonElement = button({ label: "Cancel", variant: "secondary", onClick: () => session.cancel() });
  const actions = h("div", { class: "actions" }, saveTo, reason, cancel, save);

  // The export tracker: one status line in the Queue's style, and the 4px bar. It stays after
  // the export ends, so the result is read where the progress was.
  const bar = progressBar(null);
  const stageText = h("span", { class: "job-status", attrs: { role: "status", "aria-live": "polite" } });
  const running = h("div", { class: "workshop-progress", attrs: { "data-status": "running" } }, stageText, bar);
  const outcome = h("div", { class: "workshop-outcome" });

  async function start() {
    // A long run asks first: it is cheap to say no now and costly to find out in ten minutes.
    const long = session.estimate ? longRunMessage(session.estimate) : null;
    if (long) {
      const choice = await confirmSheet("This will take a while", long, [{ label: `Start ${session.format.toUpperCase()}`, value: "start", variant: "primary" }]);
      if (choice !== "start") return;
    }
    void session.start();
  }

  let lastOutcome = "";
  function paintOutcome() {
    const state = session.state;
    const key = state.phase === "running" ? "running" : JSON.stringify(state);
    if (key === lastOutcome) return;
    lastOutcome = key;
    if (state.phase === "saved") {
      replace(
        outcome,
        h(
          "div",
          { class: "notice", attrs: { role: "status" } },
          h("span", { class: "notice-text" }, h("strong", { text: state.filename }), state.note ? h("span", { text: state.note }) : null),
          button({ label: "Show in folder", variant: "secondary", size: "small", onClick: () => void api.openOutputDir().catch((e) => toast(errorMessage(e), { tone: "error" })) }),
        ),
      );
    } else if (state.phase === "failed") {
      const detail = state.detail;
      replace(
        outcome,
        h("div", { class: "notice notice-error", attrs: { role: "alert" } }, h("span", { class: "notice-text", text: state.message })),
        detail
          ? h(
              "details",
              { class: "job-error" },
              h("summary", { text: "Show the full message" }),
              h("pre", { class: "job-error-text", text: detail }),
              button({
                label: "Copy",
                variant: "plain",
                size: "small",
                onClick: () =>
                  void navigator.clipboard.writeText(detail).then(
                    () => toast("Copied the message."),
                    () => toast("Couldn't copy the message. Select it and press Ctrl+C.", { tone: "error" }),
                  ),
              }),
            )
          : null,
      );
    } else if (state.phase === "unsupported") {
      replace(
        outcome,
        h(
          "div",
          { class: "notice", attrs: { role: "status" } },
          h("span", { class: "notice-text", text: state.message }),
          button({
            label: "Save as ZIP",
            variant: "secondary",
            size: "small",
            onClick: () => {
              session.setFormat("zip");
              void start();
            },
          }),
        ),
      );
    } else {
      replace(outcome);
    }
  }

  // What the bar showed last, so a stage change or a jump snaps and a continuing one glides.
  let shown: ProgressState | undefined;
  let lastPhase = session.state.phase;
  function paintTracker() {
    const state = session.state;
    const tracking = state.phase === "running" || state.phase === "saved" || state.phase === "failed" || state.phase === "cancelled";
    running.hidden = !tracking;
    if (!tracking) {
      shown = undefined;
      lastPhase = state.phase;
      return;
    }
    const changed = lastPhase !== state.phase;
    lastPhase = state.phase;
    running.dataset.status = state.phase;
    let fraction: number | null;
    let text: string;
    let status: string;
    if (state.phase === "running") {
      fraction = state.fraction;
      text = stageLine(state.progress);
      status = state.progress.stage;
    } else if (state.phase === "saved") {
      // 100% and green only now, when the file is in the folder.
      fraction = 1;
      text = savedLine(state.path, state.elapsedMs);
      status = "saved";
    } else {
      fraction = session.lastFraction ?? 0;
      text = state.phase === "cancelled" ? "Cancelled" : "Didn't finish";
      status = state.phase;
    }
    paintJobStatus(stageText, text, state.phase === "saved", changed, () => icon("check", 12));
    const next: ProgressState = { status, progress: fraction ?? 0, progressKnown: fraction !== null };
    setProgress(bar, fraction, progressMotion(shown, next, document.hidden));
    shown = next;
  }

  function paint() {
    const busy = session.running;
    const library = session.library;
    formatRow.hidden = !session.offersMore;
    paintFormats();
    formatHint.textContent =
      session.format === "zip"
        ? "Keeps the whole project, as Steam downloaded it."
        : session.recording && session.format === "png"
          ? "One picture of the wallpaper, after it has loaded."
          : session.recording
            ? "Wallpaper Engine plays it in the background and Claws records it. You can keep using your PC."
            : session.format === "mp4" && session.video?.copyable
              ? "Saves the wallpaper's own video file, unchanged."
              : session.format === "mp4"
                ? "Converts the wallpaper's video with FFmpeg; the project itself isn't kept."
                : "A silent looping GIF; the project itself isn't kept.";
    estimate.textContent = busy ? "" : estimateLine(session.estimate);
    estimate.hidden = !estimate.textContent;
    gifRow.hidden = !session.videoAvailable || session.format !== "gif";
    const recording = session.recording;
    const video = recording && session.format === "mp4";
    lengthRow.hidden = !video;
    customRow.hidden = !video || !session.lengthCustom;
    fpsRow.hidden = !video;
    sizeRow.hidden = !recording;
    lengthSeg.set(session.lengthCustom ? "custom" : String(session.recordLength));
    if (document.activeElement !== customLength) customLength.value = String(session.recordLength);
    lengthSeg.el.querySelectorAll("button").forEach((b) => b.setAttribute("aria-disabled", busy ? "true" : "false"));
    paintSizes();
    const fps60 = session.size?.fps.includes(60) ?? false;
    fpsSeg.set(String(session.fps));
    fpsSeg.el.querySelectorAll("button").forEach((b, index) => b.setAttribute("aria-disabled", busy || (index === 1 && !fps60) ? "true" : "false"));
    fpsSeg.el.querySelectorAll("button")[1]?.setAttribute("title", fps60 ? "" : "60 fps at this size needs a hardware video encoder");
    const notes = session.record && recording ? recordNotes(session.record, session.format, session.size) : [];
    replace(recordNote, ...notes.map((note) => h("p", { class: "row-hint", text: note })));
    recordNote.hidden = notes.length === 0;
    gifSeg.set(String(session.gifSeconds));
    gifSeg.el.querySelectorAll("button").forEach((b) => b.setAttribute("aria-disabled", busy ? "true" : "false"));

    libraryText.textContent = libraryLine(library, session.item.fileSize, session.watching);
    // A scene or web wallpaper that cannot be recorded says why in one line.
    if (library?.found && session.record && !session.record.available && session.record.reason) {
      libraryText.textContent = `${libraryText.textContent} ${session.record.reason}`;
    }
    const waiting = library !== null && library.steam && !library.found && library.state !== "downloading";
    subscribe.hidden = !library || !waiting || session.watching || !library.wallpaperEngine;
    viewInSteam.hidden = !library || (library.steam && (library.found || library.wallpaperEngine || library.state === "downloading"));
    stopWatching.hidden = !session.watching;

    const missing = missingForStart(library);
    reason.textContent = busy ? "" : (missing ?? "");
    reason.hidden = busy || !missing;
    saveTo.textContent = `Saves to ${folderName(store.settings.outputDir)}`;
    saveTo.hidden = Boolean(missing) && !busy;
    save.hidden = busy;
    save.disabled = Boolean(missing);
    const saveLabel = save.querySelector("span");
    if (saveLabel) saveLabel.textContent = `Save ${session.format.toUpperCase()}`;
    cancel.hidden = !busy;

    paintTracker();
    paintOutcome();
  }

  const el = h("section", { class: "workshop-panel" }, preview, required, sectionTitle("Options"), options, recordNote, running, actions, outcome);
  const off = [session.on(paint), store.on("settings", paint)];
  paint();
  if (!session.library) void session.checkLibrary();
  else void session.refreshEstimate();
  return {
    el,
    dispose: () => off.forEach((stop) => stop()),
  };
}
