import { api, errorMessage } from "../api";
import { formatBytes, folderName } from "../format";
import { store } from "../store";
import { button, segmented, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { group, progressBar, row, sectionTitle, setProgress } from "../ui/layout";
import { missingForStart, stageText, type ExportFormat, type WorkshopSession } from "../workshop-session";
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

  // Format. ZIP keeps the project and works for every item; a video project can also be
  // converted with FFmpeg to a playable MP4 or a GIF.
  const formatSeg = segmented<ExportFormat>(
    "Save as",
    [
      { value: "zip", label: "ZIP", title: "The whole project, as Steam downloaded it" },
      { value: "mp4", label: "MP4", title: "H.264 video with its audio, playable anywhere" },
      { value: "gif", label: "GIF", title: "A silent looping GIF" },
    ],
    session.format,
    (next) => session.setFormat(next),
  );
  const formatHint = h("span", { class: "row-note" });
  const formatRow = row("Save as", formatSeg.el, formatHint);

  const options = group(libraryRow, formatRow);

  // Actions and status
  const reason = h("span", { class: "row-hint workshop-reason" });
  const saveTo = h("span", { class: "save-to" });
  const save: HTMLButtonElement = button({ label: "Save ZIP", variant: "primary", onClick: () => start() });
  const cancel: HTMLButtonElement = button({ label: "Cancel", variant: "secondary", onClick: () => session.cancel() });
  const actions = h("div", { class: "actions" }, saveTo, reason, cancel, save);
  const bar = progressBar(null);
  const stageLine = h("span", { class: "row-hint", attrs: { role: "status", "aria-live": "polite" } });
  const running = h("div", { class: "workshop-progress" }, stageLine, bar);
  const outcome = h("div", { class: "workshop-outcome" });

  function start() {
    void session.start();
  }

  let lastOutcome = "";
  function paintOutcome() {
    const state = session.state;
    const key = JSON.stringify(state);
    if (key === lastOutcome) return;
    lastOutcome = key;
    if (state.phase === "saved") {
      replace(
        outcome,
        h(
          "div",
          { class: "notice", attrs: { role: "status" } },
          h("span", { class: "notice-text", text: `Saved ${state.filename}.` }),
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
    } else if (state.phase === "cancelled") {
      replace(outcome, h("p", { class: "footnote", attrs: { role: "status" }, text: "Cancelled." }));
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
              start();
            },
          }),
        ),
      );
    } else {
      replace(outcome);
    }
  }

  function paint() {
    const busy = session.running;
    const library = session.library;
    formatRow.hidden = !session.videoAvailable;
    formatSeg.set(session.format);
    formatSeg.el.querySelectorAll("button").forEach((b) => ((b as HTMLButtonElement).disabled = busy));
    formatHint.textContent =
      session.format === "zip"
        ? "Keeps the whole project, as Steam downloaded it."
        : "Converts the wallpaper's video with FFmpeg; the project itself isn't kept.";

    libraryText.textContent = !library
      ? "Checking…"
      : !library.steam
        ? "Claws needs Steam with Wallpaper Engine installed on this PC, and nothing else."
        : library.found
          ? library.video
            ? "Video found in your Steam library."
            : "Project found. Claws can save a ZIP. MP4 and GIF need a standalone video."
          : session.watching
            ? "Waiting for Steam to download it…"
            : "Not in your library yet.";
    subscribe.hidden = !library?.steam || library.found || session.watching;
    viewInSteam.hidden = !library || library.steam;
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

    running.hidden = !busy;
    if (session.state.phase === "running") {
      stageLine.textContent = stageText(session.state.stage, session.state.percent, session.format);
      setProgress(bar, session.state.percent === null ? null : session.state.percent / 100);
    }
    paintOutcome();
  }

  const el = h("section", { class: "workshop-panel" }, preview, required, sectionTitle("Options"), options, running, actions, outcome);
  const off = [session.on(paint), store.on("settings", paint)];
  paint();
  if (!session.library) void session.checkLibrary();
  return {
    el,
    dispose: () => off.forEach((stop) => stop()),
  };
}
