import { api, errorMessage } from "../api";
import { formatBytes, folderName } from "../format";
import { store } from "../store";
import { button, segmented, toast, toggle } from "../ui/components";
import { h, replace } from "../ui/dom";
import { group, progressBar, row, sectionTitle, setProgress } from "../ui/layout";
import { missingForStart, stageText, type WorkshopSession, type WorkshopSource } from "../workshop-session";
import { thumbnailEl } from "./download-shared";

export interface WorkshopView {
  el: HTMLElement;
  dispose(): void;
}

const onSaveError = (message: string) => toast(message, { tone: "error" });

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

  // Source
  const sourceSeg = segmented<WorkshopSource>(
    "Source",
    [
      { value: "library", label: "Steam library" },
      { value: "steamcmd", label: "SteamCMD" },
    ],
    session.source,
    (next) => session.setSource(next),
  );
  const sourceHint = h("span");

  // Steam library
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
  const stopWatching = button({ label: "Stop waiting", variant: "plain", size: "small", onClick: () => session.stopWatching() });
  const libraryRow = row("Steam library", h("div", { class: "inline" }, libraryText, subscribe, stopWatching));

  // SteamCMD
  const steamcmdText = h("span", { class: "path" });
  const chooseLabel = () => (store.settings.steamcmdPath ? "Change…" : "Choose…");
  const choose: HTMLButtonElement = button({
    label: chooseLabel(),
    variant: "secondary",
    size: "small",
    onClick: async () => {
      choose.disabled = true;
      try {
        const path = await api.chooseSteamcmd();
        if (path) store.updateSettings({ steamcmdPath: path }, onSaveError);
      } catch (error) {
        toast(errorMessage(error), { tone: "error" });
      } finally {
        choose.disabled = false;
        paint();
      }
    },
  });
  const steamcmdRow = row("SteamCMD", h("div", { class: "inline" }, steamcmdText, choose));
  const account = h("input", {
    class: "text-input",
    attrs: { id: "workshop-account", type: "text", maxlength: 64, spellcheck: false, autocomplete: "username", value: store.settings.steamAccount },
  });
  account.addEventListener("input", () => paint());
  account.addEventListener("change", () => store.updateSettings({ steamAccount: account.value.trim() }, onSaveError));
  account.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !save.disabled) {
      event.preventDefault();
      start();
    }
  });
  const signIn: HTMLButtonElement = button({
    label: "Sign in to Steam…",
    variant: "secondary",
    size: "small",
    onClick: () => {
      void api.signInSteamcmd(account.value.trim()).then(
        () => toast("Sign in in the SteamCMD window, then type quit."),
        (error) => toast(errorMessage(error), { tone: "error" }),
      );
    },
  });
  const accountRow = row(
    "Steam account",
    h("div", { class: "inline" }, account, signIn),
    "The account that owns Wallpaper Engine. Sign in once; SteamCMD remembers it.",
    "workshop-account",
  );
  const cleanupRow = row(
    "Remove SteamCMD's copy",
    toggle("Remove SteamCMD's copy after saving", store.settings.removeSteamcmdCopy, (value) => store.updateSettings({ removeSteamcmdCopy: value }, onSaveError)),
    "Deletes the wallpaper from SteamCMD's folder once the ZIP is saved.",
  );

  const options = group(row("Source", sourceSeg.el, sourceHint), libraryRow, steamcmdRow, accountRow, cleanupRow);

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
    if (account.value.trim() !== store.settings.steamAccount) store.updateSettings({ steamAccount: account.value.trim() }, onSaveError);
    void session.start(account.value);
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
    } else {
      replace(outcome);
    }
  }

  function paint() {
    const busy = session.running;
    const library = session.library;
    const fromLibrary = session.source === "library";
    sourceSeg.set(session.source);
    sourceSeg.el.querySelectorAll("button").forEach((b) => ((b as HTMLButtonElement).disabled = busy));
    sourceHint.textContent = fromLibrary
      ? "Uses the copy Steam downloads when you subscribe. No sign-in in Claws."
      : "Downloads with SteamCMD, signed in with your Steam account.";

    libraryRow.hidden = !fromLibrary;
    libraryText.textContent = !library
      ? "Checking…"
      : !library.steam
        ? "Steam isn't installed for this Windows user."
        : library.found
          ? "Found in your Steam library."
          : session.watching
            ? "Waiting for Steam to download it…"
            : "Not in your library yet.";
    subscribe.hidden = !library?.steam || library.found || session.watching;
    stopWatching.hidden = !session.watching;

    steamcmdRow.hidden = accountRow.hidden = cleanupRow.hidden = fromLibrary;
    const path = store.settings.steamcmdPath;
    steamcmdText.textContent = path || "Not chosen";
    steamcmdText.title = path;
    choose.querySelector("span")!.textContent = chooseLabel();
    choose.disabled = busy;
    account.disabled = busy;
    signIn.disabled = busy || !path || !account.value.trim();

    const missing = missingForStart(session.source, library, path, account.value);
    reason.textContent = busy ? "" : (missing ?? "");
    reason.hidden = busy || !missing;
    saveTo.textContent = `Saves to ${folderName(store.settings.outputDir)}`;
    saveTo.hidden = Boolean(missing) && !busy;
    save.hidden = busy;
    save.disabled = Boolean(missing);
    cancel.hidden = !busy;

    running.hidden = !busy;
    if (session.state.phase === "running") {
      stageLine.textContent = stageText(session.state.stage, session.state.percent);
      setProgress(bar, session.state.percent === null ? null : session.state.percent / 100);
    }
    paintOutcome();
  }

  const el = h("section", { class: "workshop-panel" }, preview, required, sectionTitle("Options"), options, running, actions, outcome);
  const off = [session.on(paint), store.on("settings", paint)];
  paint();
  if (!session.library) void session.checkLibrary(true);
  return {
    el,
    dispose: () => off.forEach((stop) => stop()),
  };
}
