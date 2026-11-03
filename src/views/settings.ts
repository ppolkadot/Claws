import { open } from "@tauri-apps/plugin-dialog";

import { api, errorMessage, type Backdrop, type CookieKind, type Theme } from "../api";
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
  const s = () => store.settings;
  const update = (patch: Parameters<typeof store.updateSettings>[0]) => store.updateSettings(patch, onError);

  async function chooseFolder() {
    const picked = await open({ directory: true, defaultPath: s().outputDir || undefined, title: "Choose where downloads are saved" });
    if (typeof picked === "string") {
      update({ outputDir: picked });
      paintLocation();
    }
  }
  async function chooseCookies() {
    const picked = await open({ multiple: false, filters: [{ name: "Cookies", extensions: ["txt"] }], title: "Choose a cookies.txt file" });
    if (typeof picked === "string") {
      update({ cookieSource: { kind: "file", value: picked } });
      paintCookies();
    }
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

  const intensityValue = h("span", { class: "value", text: `${s().glassIntensity}%` });
  const bundled = [...document.fonts].some((f) => f.family.replace(/"/g, "") === "Legacy UI Display" && f.status === "loaded");
  const Legacy UI = bundled || fontInstalled("Legacy UI Display");
  const fontRow = row(
    "Font",
    Legacy UI
      ? h("span", { class: "font-status" }, h("span", { class: "tool-dot", attrs: { "data-state": "ok" } }), "Legacy UI Display")
      : button({ label: "Get Legacy UI…", variant: "secondary", size: "small", onClick: () => void api.openFontPage().catch((e) => onError(errorMessage(e))) }),
    bundled
      ? "Built into this copy of Claws."
      : Legacy UI
        ? "Installed on this PC and used throughout Claws."
        : "Legacy UI Display is not available, so Claws uses Segoe UI Variable. Build the fonts (see README) or install Legacy UI from Apple, then restart Claws.",
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
    if (source.kind === "browser") {
      replace(
        cookieDetail,
        row(
          "Browser",
          chipRadio("Browser", BROWSERS, source.value || "firefox", (value) => update({ cookieSource: { kind: "browser", value } })),
          "Firefox is the most reliable. Close Chrome, Edge and similar browsers first, or they may block access to their cookies.",
        ),
      );
      if (!source.value) update({ cookieSource: { kind: "browser", value: "firefox" } });
    } else if (source.kind === "file") {
      replace(
        cookieDetail,
        row(
          "Cookies file",
          h(
            "div",
            { class: "inline" },
            h("span", { class: "path", text: source.value || "No file chosen", attrs: { title: source.value } }),
            button({ label: "Choose…", variant: "secondary", size: "small", onClick: () => void chooseCookies() }),
          ),
          "A cookies.txt exported from your browser.",
        ),
      );
    } else {
      replace(cookieDetail);
    }
  };
  paintCookies();

  const cookies = group(
    row(
      "Use cookies from",
      segmented<CookieKind>(
        "Cookie source",
        [
          { value: "none", label: "None" },
          { value: "browser", label: "Browser" },
          { value: "file", label: "File" },
        ],
        s().cookieSource.kind,
        (kind) => {
          if (kind === "browser") update({ cookieSource: { kind, value: s().cookieSource.value || "firefox" } });
          else if (kind === "file") {
            update({ cookieSource: { kind, value: "" } });
            void chooseCookies();
          } else update({ cookieSource: { kind, value: "" } });
          paintCookies();
        },
      ).el,
      "Lets Claws download videos that need you to be signed in. Cookies are only read when needed and are never sent anywhere except the site itself.",
    ),
    cookieDetail,
  );

  const toolsGroup = h("div", { class: "tools" });
  const paintTools = () => {
    const { ytdlp, ffmpeg } = store.tools;
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
      ),
    );
    toolsGroup.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = ytdlpBusy || ffmpegBusy));
  };
  const offTools = [store.on("tools", paintTools), store.on("toolProgress", paintTools)];
  paintTools();

  container.append(
    h("div", { class: "settings" },
      sectionTitle("Downloads"), downloads,
      sectionTitle("Appearance"), appearance,
      sectionTitle("Sign-in"), cookies,
      sectionTitle("Tools"), toolsGroup,
      h("p", { class: "footnote", text: "Claws uses yt-dlp and ffmpeg and runs entirely on this PC. Only download content you have the right to save." }),
    ),
  );

  return () => offTools.forEach((off) => off());
};
