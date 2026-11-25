import { open } from "@tauri-apps/plugin-dialog";

import { api, errorMessage, type Backdrop, type CookieKind, type QuickOpenShortcut, type Theme } from "../api";
import type { ToolName } from "../setup";
import { store } from "../store";
import { button, chipRadio, segmented, slider, stepper, toast, toggle } from "../ui/components";
import { h, replace } from "../ui/dom";
import { group, progressBar, row, sectionTitle, setProgress } from "../ui/layout";
import type { MountView } from "./shell";

const BROWSERS = [
  { value: "firefox", label: "Firefox" },
  { value: "chrome", label: "Chrome" },
  { value: "edge", label: "Edge" },
  { value: "brave", label: "Brave" },
  { value: "opera", label: "Opera" },
  { value: "vivaldi", label: "Vivaldi" },
];

// Mirrors SHORTCUTS in src-tauri/src/settings.rs.
const SHORTCUTS: { value: QuickOpenShortcut; label: string }[] = [
  { value: "ctrl+alt+d", label: "Ctrl+Alt+D" },
  { value: "ctrl+alt+space", label: "Ctrl+Alt+Space" },
  { value: "ctrl+shift+space", label: "Ctrl+Shift+Space" },
  { value: "off", label: "Off" },
];

const SPEED_LIMITS: { value: number; label: string }[] = [
  { value: 0, label: "No limit" },
  { value: 1024, label: "1 MB/s" },
  { value: 2048, label: "2 MB/s" },
  { value: 5120, label: "5 MB/s" },
  { value: 10240, label: "10 MB/s" },
  { value: 25600, label: "25 MB/s" },
];

/** The offered limits, plus a saved value that is not one of them (a hand-edited file). */
function speedLimits(current: number): { value: number; label: string }[] {
  if (SPEED_LIMITS.some((option) => option.value === current)) return SPEED_LIMITS;
  return [...SPEED_LIMITS, { value: current, label: `${current} KiB/s` }];
}

const onError = (message: string) => toast(message, { tone: "error" });

export const mountSettings: MountView = (container, ctx) => {
  ctx.setActions([]);
  let cleanup = buildSettings(container);
  // A rejected save puts fields back; rebuild so every control shows the saved value.
  const offReverted = store.on("settingsReverted", () => {
    cleanup();
    container.replaceChildren();
    cleanup = buildSettings(container);
  });
  return () => {
    offReverted();
    cleanup();
  };
};

function buildSettings(container: HTMLElement): () => void {
  const s = () => store.settings;
  const update = (patch: Parameters<typeof store.updateSettings>[0]) => store.updateSettings(patch, onError);

  async function chooseFolder() {
    const picked = await open({ directory: true, defaultPath: s().outputDir || undefined, title: "Choose where downloads are saved" });
    if (typeof picked === "string") {
      update({ outputDir: picked });
      paintLocation();
    }
  }
  // File only takes effect once a file is chosen; an empty path would be rejected.
  let shownKind: CookieKind = s().cookieSource.kind;
  // Kept across switches, so Browser never inherits a file path.
  let lastBrowser = s().cookieSource.kind === "browser" && s().cookieSource.value ? s().cookieSource.value : "firefox";

  async function chooseCookies() {
    const picked = await open({ multiple: false, filters: [{ name: "Cookies", extensions: ["txt"] }], title: "Choose a cookies.txt file" });
    if (typeof picked === "string") {
      shownKind = "file";
      update({ cookieSource: { kind: "file", value: picked } });
    } else if (s().cookieSource.kind !== "file") {
      // Cancelled with no file in use: show what is actually saved again.
      shownKind = s().cookieSource.kind;
      cookieSegment.set(shownKind);
    }
    paintCookies();
  }

  const location = h("span", { class: "path", attrs: { title: "" } });
  const paintLocation = () => {
    location.textContent = s().outputDir;
    location.title = s().outputDir;
  };
  paintLocation();

  const template = h("input", {
    class: "text-input mono",
    attrs: { id: "settings-filename", type: "text", value: s().filenameTemplate, spellcheck: false },
  });
  template.addEventListener("change", () => update({ filenameTemplate: template.value }));

  const downloads = group(
    row(
      "Save to",
      h(
        "div",
        { class: "inline" },
        location,
        button({ label: "Choose…", variant: "secondary", size: "small", onClick: () => void chooseFolder() }),
        button({ label: "Show", variant: "plain", size: "small", onClick: () => void api.openOutputDir().catch((e) => onError(errorMessage(e))) }),
      ),
    ),
    row("Filename template", template, h("span", {}, "Use ", h("code", { text: "%(title)s" }), " for the title and ", h("code", { text: "%(id)s" }), " for the source ID."), template.id),
    row("Simultaneous downloads", stepper("Simultaneous downloads", s().maxConcurrent, 1, 8, (v) => update({ maxConcurrent: v }))),
    row(
      "Simultaneous conversions",
      stepper("Simultaneous conversions", s().maxProcessing, 1, 8, (v) => update({ maxProcessing: v })),
      "Downloads wait when this many are merging or converting.",
    ),
    row(
      "Speed limit",
      chipRadio<number>("Speed limit", speedLimits(s().speedLimitKib), s().speedLimitKib, (v) => update({ speedLimitKib: v })),
      "Applies to each download. Takes effect on the next start or resume.",
    ),
    row(
      "Embed metadata",
      toggle("Embed metadata", s().embedMetadata, (v) => update({ embedMetadata: v })),
      "Adds title, uploader, date and chapters. May rewrite the file after download, which takes longer on large files.",
    ),
    row(
      "Prefer matching source audio",
      toggle("Prefer matching source audio", s().audioSelection === "preferMatching", (v) => update({ audioSelection: v ? "preferMatching" : "best" })),
      "Saves audio already in the chosen format without converting it. Bitrate may differ from the site's best stream.",
    ),
  );

  const quickAccess = group(
    row(
      "Quick Open",
      chipRadio<QuickOpenShortcut>("Quick Open shortcut", SHORTCUTS, s().quickOpenShortcut, (value) => update({ quickOpenShortcut: value })),
      "Brings Claws forward from any app, and looks up a link that is on the clipboard. Press it again to put Claws away.",
    ),
    row(
      "Notify when a download finishes",
      toggle("Notify when a download finishes", s().notifyWhenDone, (value) => update({ notifyWhenDone: value })),
      "When a download finishes or fails while Claws is in the background.",
    ),
    row(
      "Keep running when closed",
      toggle("Keep running when closed", s().keepRunning, (value) => update({ keepRunning: value })),
      "Closing the window leaves Claws in the notification area, so downloads continue and Quick Open keeps working. Quit from its icon there.",
    ),
    row(
      "Open at login",
      toggle("Open at login", s().openAtLogin, (value) => update({ openAtLogin: value })),
      "Starts in the notification area.",
    ),
  );

  const intensityValue = h("span", { class: "value", text: `${s().glassIntensity}%` });
  const loaded = (family: string) => [...document.fonts].some((f) => f.family.replace(/"/g, "") === family && f.status === "loaded");
  const polaris = loaded("Claws Polaris");
  function fontStatusRow(inUse: boolean): HTMLElement {
    return row(
      "Font",
      inUse
        ? h("span", { class: "row-note", text: "Salted Flutters Polaris" })
        : button({ label: "Font details…", variant: "secondary", size: "small", onClick: () => void api.openFontPage().catch((e) => onError(errorMessage(e))) }),
      inUse
        ? undefined
        : "Salted Flutters Polaris is unavailable. Reinstall Claws to restore the bundled font.",
    );
  }
  const fontRow = fontStatusRow(polaris);
  // Startup waits at most 800 ms for the font; rebuild this row once it finishes loading.
  if (!polaris) {
    void document.fonts.ready.then(() => {
      if (!loaded("Claws Polaris") || !fontRow.isConnected) return;
      fontRow.replaceWith(fontStatusRow(true));
    });
  }
  const appearance = group(
    row(
      "Theme",
      segmented<Theme>(
        "Theme",
        [
          { value: "system", label: "System" },
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ],
        s().theme,
        (value) => update({ theme: value }),
      ).el,
    ),
    row(
      "Window material",
      segmented<Backdrop>(
        "Window material",
        [
          { value: "glass", label: "Glass" },
          { value: "acrylic", label: "Acrylic" },
          { value: "mica", label: "Mica" },
          { value: "none", label: "Solid" },
        ],
        s().backdrop === "tabbed" ? "mica" : s().backdrop,
        (value) => update({ backdrop: value }),
      ).el,
      "Glass shows your desktop through the window, blurred. Acrylic and Mica are the Windows materials and turn flat while the window is in the background.",
    ),
    row(
      "Transparency",
      h(
        "div",
        { class: "inline" },
        slider("Transparency", s().glassIntensity, (value) => {
          intensityValue.textContent = `${value}%`;
          update({ glassIntensity: value });
        }),
        intensityValue,
      ),
      "Lower this if text is hard to read over a busy wallpaper.",
    ),
    fontRow,
  );

  const cookieDetail = h("div", { class: "cookie-detail" });
  const paintCookies = () => {
    const source = s().cookieSource;
    if (shownKind === "browser") {
      const browser = source.kind === "browser" && source.value ? source.value : lastBrowser;
      replace(
        cookieDetail,
        row(
          "Browser",
          chipRadio("Browser", BROWSERS, browser, (value) => {
            lastBrowser = value;
            update({ cookieSource: { kind: "browser", value } });
          }),
          "Firefox is the most reliable. Close Chrome, Edge and similar browsers first, or they may block access to their cookies.",
        ),
      );
    } else if (shownKind === "file") {
      const path = source.kind === "file" ? source.value : "";
      replace(
        cookieDetail,
        row(
          "Cookies file",
          h(
            "div",
            { class: "inline" },
            h("span", { class: "path", text: path || "No file chosen", attrs: { title: path } }),
            button({ label: "Choose…", variant: "secondary", size: "small", onClick: () => void chooseCookies() }),
          ),
          path ? undefined : "Choose a cookies.txt file exported from your browser.",
        ),
      );
    } else {
      replace(cookieDetail);
    }
  };
  paintCookies();

  const cookieSegment = segmented<CookieKind>(
    "Cookie source",
    [
      { value: "none", label: "None" },
      { value: "browser", label: "Browser" },
      { value: "file", label: "File" },
    ],
    shownKind,
    (kind) => {
      shownKind = kind;
      if (kind === "browser") update({ cookieSource: { kind, value: lastBrowser } });
      else if (kind === "file") {
        if (s().cookieSource.kind !== "file") void chooseCookies();
      } else update({ cookieSource: { kind: "none", value: "" } });
      paintCookies();
    },
  );
  const cookies = group(
    row(
      "Use cookies from",
      cookieSegment.el,
      "For videos that need you to be signed in. yt-dlp sends each cookie only to the site it belongs to.",
    ),
    cookieDetail,
  );

  // Wallpaper Engine: what the SteamCMD source uses (the Steam library source needs nothing).
  const steamcmdPath = h("span", { class: "path" });
  const installSteamcmd = button({
    label: "Install",
    variant: "secondary",
    size: "small",
    onClick: () => void store.runTool("steamcmd", api.installSteamcmd),
  });
  // "Clear" goes back to Claws' own copy (or none).
  const clearSteamcmd = button({
    label: "Clear",
    variant: "plain",
    size: "small",
    onClick: () => void store.setSteamcmd("", onError),
  });
  const paintSteam = () => {
    const tool = store.tools.steamcmd;
    const busy = Boolean(store.toolBusy.steamcmd);
    const progress = store.toolProgress.steamcmd;
    const pct = progress && progress.total > 0 ? ` ${Math.round((progress.received / progress.total) * 100)}%` : "…";
    steamcmdPath.textContent = busy
      ? progress?.stage === "update" ? `Updating${pct}` : progress?.stage === "extract" ? "Checking Valve's signature…" : `Downloading${pct}`
      : tool ? (tool.managed ? "Installed by Claws" : tool.path) : "Not installed";
    steamcmdPath.title = tool?.path ?? "";
    installSteamcmd.hidden = Boolean(tool) && !busy;
    installSteamcmd.disabled = busy;
    clearSteamcmd.hidden = !s().steamcmdPath;
    const error = store.toolError.steamcmd;
    steamcmdHint.textContent = error ?? (tool ? "Only needed for wallpapers you haven't subscribed to in Steam." : "Downloads Valve's SteamCMD into Claws' tools folder. Only needed for wallpapers you haven't subscribed to in Steam.");
  };
  const steamcmdHint = h("span");
  const steamAccount = h("input", {
    class: "text-input",
    attrs: { id: "settings-steam-account", type: "text", maxlength: 64, spellcheck: false, autocomplete: "username", value: s().steamAccount },
  });
  steamAccount.addEventListener("change", () => update({ steamAccount: steamAccount.value.trim() }));
  const wallpaper = group(
    row(
      "SteamCMD",
      h(
        "div",
        { class: "inline" },
        steamcmdPath,
        installSteamcmd,
        button({
          label: "Choose…",
          variant: "plain",
          size: "small",
          onClick: () =>
            void api.chooseSteamcmd().then(
              (path) => (path ? store.setSteamcmd(path, onError) : undefined),
              (e) => onError(errorMessage(e)),
            ),
        }),
        clearSteamcmd,
      ),
      steamcmdHint,
    ),
    row("Steam account", steamAccount, "The account that owns Wallpaper Engine.", steamAccount.id),
    row(
      "Remove SteamCMD's copy",
      toggle("Remove SteamCMD's copy after saving", s().removeSteamcmdCopy, (v) => update({ removeSteamcmdCopy: v })),
      "Deletes the wallpaper from SteamCMD's folder once the ZIP is saved.",
    ),
  );
  paintSteam();

  const toolsGroup = h("div", { class: "tools" });
  const paintTools = () => {
    const { ytdlp, ffmpeg, deno, jsRuntime } = store.tools;
    const denoBusy = Boolean(store.toolBusy.deno);
    const ytdlpBusy = Boolean(store.toolBusy.ytdlp);
    const ffmpegBusy = Boolean(store.toolBusy.ffmpeg);

    const progressFor = (tool: ToolName) => {
      const p = store.toolProgress[tool];
      const fraction = p && p.total > 0 ? p.received / p.total : null;
      const bar = progressBar(fraction);
      setProgress(bar, fraction);
      return bar;
    };

    const upToDate = ytdlp && store.tools.latestYtdlp === ytdlp.version;
    const ytdlpHint = store.toolError.ytdlp
      ? store.toolError.ytdlp
      : ytdlp
        ? upToDate
          ? "You have the newest version."
          : ytdlp.managed
            ? "Sites change often. Update when a download stops working."
            : "Found on your PATH. Updating installs a copy that Claws manages."
        : store.toolsKnown
          ? "Not installed yet."
          : "Checking…";

    const ffmpegHint = store.toolError.ffmpeg
      ? store.toolError.ffmpeg
      : ffmpeg
        ? ffmpeg.managed
          ? "Installed by Claws."
          : "Found on your PATH."
        : store.toolsKnown
          ? "Needed to merge HD video with audio and to convert audio. Not installed."
          : "Checking…";

    replace(
      toolsGroup,
      group(
        row(
          `yt-dlp${ytdlp ? `  ${ytdlp.version}` : ""}`,
          h(
            "div",
            { class: "tool-control" },
            ytdlpBusy ? progressFor("ytdlp") : null,
            button({
              label: ytdlp ? "Check for updates" : "Install",
              variant: "secondary",
              size: "small",
              onClick: () => void store.runTool("ytdlp", ytdlp ? api.updateYtdlp : api.installYtdlp),
            }),
          ),
          ytdlpHint,
        ),
        row(
          `ffmpeg${ffmpeg ? `  ${ffmpeg.version}` : ""}`,
          ffmpeg
            ? null
            : h(
                "div",
                { class: "tool-control" },
                ffmpegBusy ? progressFor("ffmpeg") : null,
                button({ label: "Install", variant: "secondary", size: "small", onClick: () => void store.runTool("ffmpeg", api.installFfmpeg) }),
              ),
          ffmpegHint,
        ),
        row(
          deno ? `Deno ${deno.version}` : jsRuntime ? jsRuntime : "JavaScript runtime",
          !store.toolsKnown || jsRuntime
            ? null
            : h(
                "div",
                { class: "tool-control" },
                denoBusy ? progressFor("deno") : null,
                button({ label: "Install", variant: "secondary", size: "small", onClick: () => void store.runTool("deno", api.installDeno) }),
              ),
          store.toolError.deno
            ? store.toolError.deno
            : !store.toolsKnown
              ? "Checking…"
              : deno?.managed
                ? "Installed by Claws. yt-dlp uses Deno to read sites such as YouTube."
                : jsRuntime
                  ? `Found on your PATH. yt-dlp uses ${jsRuntime} to read sites such as YouTube.`
                  : "YouTube and some other sites need a JavaScript runtime.",
        ),
      ),
    );
    toolsGroup.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = ytdlpBusy || ffmpegBusy || denoBusy));
  };
  const offTools = [store.on("tools", paintTools), store.on("toolProgress", paintTools), store.on("tools", paintSteam), store.on("toolProgress", paintSteam)];
  paintTools();

  container.append(
    h("div", { class: "settings" },
      sectionTitle("Downloads"), downloads,
      sectionTitle("Quick access"), quickAccess,
      sectionTitle("Appearance"), appearance,
      sectionTitle("Sign-in"), cookies,
      sectionTitle("Wallpaper Engine"), wallpaper,
      sectionTitle("Tools"), toolsGroup,
      h("p", { class: "footnote", text: "Only download content you have the right to save." }),
    ),
  );

  return () => offTools.forEach((off) => off());
}
