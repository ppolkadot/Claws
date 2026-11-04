import { open } from "@tauri-apps/plugin-dialog";

import { api, errorMessage, type Backdrop, type CookieKind, type QuickOpenShortcut, type Theme } from "../api";
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

const onError = (message: string) => toast(message, { tone: "error" });

/** True when the named font is installed: text set in it measures differently from the fallback. */
function fontInstalled(family: string): boolean {
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) return false;
  const sample = "Claws mmmmmmmmmwwwwwww 0123456789";
  return ["monospace", "serif"].some((fallback) => {
    ctx.font = `40px ${fallback}`;
    const base = ctx.measureText(sample).width;
    ctx.font = `40px "${family}", ${fallback}`;
    return ctx.measureText(sample).width !== base;
  });
}

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
    attrs: { type: "text", value: s().filenameTemplate, "aria-label": "Filename template", spellcheck: false },
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
    row("Filename", template, h("span", {}, "yt-dlp output template. ", h("code", { text: "%(title)s" }), " and ", h("code", { text: "%(id)s" }), " are the usual parts.")),
    row("Simultaneous downloads", stepper("Simultaneous downloads", s().maxConcurrent, 1, 8, (v) => update({ maxConcurrent: v }))),
    row(
      "Embed metadata",
      toggle("Embed metadata", s().embedMetadata, (v) => update({ embedMetadata: v })),
      "Writes title, uploader and date into the file. It rewrites the file once more, which adds a few seconds on long videos.",
    ),
  );

  const quickAccess = group(
    row(
      "Quick Open",
      chipRadio<QuickOpenShortcut>("Quick Open shortcut", SHORTCUTS, s().quickOpenShortcut, (value) => update({ quickOpenShortcut: value })),
      "Brings Claws forward from any app, and looks up a link that is on the clipboard. Press it again to put Claws away.",
    ),
    row(
      "Keep running when closed",
      toggle("Keep running when closed", s().keepRunning, (value) => update({ keepRunning: value })),
      "Closing the window leaves Claws in the notification area, so downloads continue and Quick Open keeps working. Quit from its icon there.",
    ),
    row(
      "Open at login",
      toggle("Open at login", s().openAtLogin, (value) => update({ openAtLogin: value })),
      "Starts Claws in the notification area when you sign in to Windows, ready for Quick Open.",
    ),
  );

  const intensityValue = h("span", { class: "value", text: `${s().glassIntensity}%` });
  const loaded = (family: string) => [...document.fonts].some((f) => f.family.replace(/"/g, "") === family && f.status === "loaded");
  const family = ["Salted Flutters Polaris", "Legacy UI Display"].find((f) => loaded(f) || fontInstalled(f));
  const bundled = family !== undefined && loaded(family);
  const Legacy UI = family !== undefined;
  const fontRow = row(
    "Font",
    Legacy UI
      ? h("span", { class: "font-status" }, h("span", { class: "tool-dot", attrs: { "data-state": "ok" } }), family)
      : button({ label: "Get Legacy UI…", variant: "secondary", size: "small", onClick: () => void api.openFontPage().catch((e) => onError(errorMessage(e))) }),
    bundled
      ? "In use throughout Claws."
      : Legacy UI
        ? "Installed on this PC and used throughout Claws."
        : "Neither Salted Flutters Polaris nor Legacy UI Display is available, so Claws uses Segoe UI Variable. Build the fonts (see README) or install Legacy UI from Apple, then restart Claws.",
  );
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
      "How much of the desktop shows through. Lower it if text is hard to read over a busy wallpaper.",
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
          path ? "A cookies.txt exported from your browser." : "Choose a cookies.txt file to start using it.",
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
      "Lets Claws download videos that need you to be signed in. While a source is set, yt-dlp reads the cookies each time it looks up or downloads a link, and sends each one only to the site it belongs to.",
    ),
    cookieDetail,
  );

  const toolsGroup = h("div", { class: "tools" });
  const paintTools = () => {
    const { ytdlp, ffmpeg, jsRuntime } = store.tools;
    const ytdlpBusy = Boolean(store.toolBusy.ytdlp);
    const ffmpegBusy = Boolean(store.toolBusy.ffmpeg);

    const progressFor = (tool: "ytdlp" | "ffmpeg") => {
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
              label: ytdlp ? "Check for Updates" : "Install",
              icon: "retry",
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
                button({ label: "Install", icon: "download", variant: "secondary", size: "small", onClick: () => void store.runTool("ffmpeg", api.installFfmpeg) }),
              ),
          ffmpegHint,
        ),
        row(
          "JavaScript runtime",
          store.toolsKnown
            ? h(
                "span",
                { class: "font-status" },
                h("span", { class: "tool-dot", attrs: { "data-state": jsRuntime ? "ok" : "missing" } }),
                jsRuntime ?? "Not found",
              )
            : null,
          !store.toolsKnown
            ? "Checking…"
            : jsRuntime
              ? `Found on your PATH. yt-dlp uses ${jsRuntime} to read sites such as YouTube.`
              : "YouTube and some other sites need Deno or Node.js. Install one (deno.com is the simplest), then restart Claws.",
        ),
      ),
    );
    toolsGroup.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = ytdlpBusy || ffmpegBusy));
  };
  const offTools = [store.on("tools", paintTools), store.on("toolProgress", paintTools)];
  paintTools();

  container.append(
    h("div", { class: "settings" },
      sectionTitle("Downloads"), downloads,
      sectionTitle("Quick Access"), quickAccess,
      sectionTitle("Appearance"), appearance,
      sectionTitle("Sign-in"), cookies,
      sectionTitle("Tools"), toolsGroup,
      h("p", { class: "footnote", text: "Claws uses yt-dlp and ffmpeg and runs entirely on this PC. Only download content you have the right to save." }),
    ),
  );

  return () => offTools.forEach((off) => off());
}
