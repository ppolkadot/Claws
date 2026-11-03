import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type DownloadOptions, type ProbeResult } from "../api";
import { formatDuration, folderName } from "../format";
import { store } from "../store";
import { button, chipCheck, chipRadio, segmented, toast, toggle } from "../ui/components";
import { clear, h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { group, progressBar, row, sectionTitle, setProgress, spinner } from "../ui/layout";
import type { MountView } from "./shell";

type Phase = "idle" | "loading" | "result" | "error";

const STANDARD_HEIGHTS = [2160, 1440, 1080, 720, 480, 360, 240, 144];
const AUDIO_FORMATS = [
  { value: "mp3", label: "MP3" },
  { value: "m4a", label: "M4A" },
  { value: "opus", label: "Opus" },
  { value: "flac", label: "FLAC" },
  { value: "wav", label: "WAV" },
  { value: "best", label: "Original" },
];

function qualityChoices(heights: number[]): { value: number; label: string }[] {
  const max = heights[0] ?? 0;
  let picks = STANDARD_HEIGHTS.filter((h) => heights.some((available) => available >= h * 0.95) && h <= max * 1.05);
  if (picks.length === 0) picks = heights.slice(0, 6);
  return picks.slice(0, 7).map((value) => ({ value, label: value === 2160 ? "4K" : `${value}p` }));
}

function looksLikeUrl(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
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

  let phase: Phase = "idle";
  let result: ProbeResult | null = null;
  let errorText = "";
  let lastUrl = "";

  let mode: DownloadOptions["mode"] = "video";
  let height: number | null = null;
  let audioFormat = "mp3";
  let subs = new Set<string>();
  let embedSubs = false;
  let selected = new Set<number>();

  const input = h("input", {
    class: "url-input",
    attrs: {
      type: "url",
      placeholder: "Paste a video link",
      "aria-label": "Video link",
      autocomplete: "off",
      spellcheck: false,
    },
  });
  const goButton = button({ label: "Get Info", variant: "primary", onClick: () => void submit() });
  const pasteButton = button({
    icon: "paste",
    title: "Paste from clipboard",
    variant: "plain",
    onClick: async () => {
      try {
        const text = (await navigator.clipboard.readText()).trim();
        if (text) {
          input.value = text;
          input.focus();
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

  async function submit() {
    const url = input.value.trim();
    if (!url) {
      input.focus();
      return;
    }
    if (!looksLikeUrl(url)) {
      phase = "error";
      errorText = "That does not look like a link. Paste the full address, starting with https://";
      return render();
    }
    await store.toolsReady;
    if (!store.tools.ytdlp) {
      phase = "error";
      errorText = "yt-dlp is still being set up. Try again in a moment.";
      return render();
    }
    lastUrl = url;
    phase = "loading";
    render();
    try {
      const probed = await api.probe(url);
      if (lastUrl !== url) return; // a newer request replaced this one
      result = probed;
      mode = "video";
      height = null;
      audioFormat = "mp3";
      subs = new Set();
      embedSubs = false;
      selected = new Set(probed.entries.map((_, i) => i));
      phase = "result";
    } catch (error) {
      if (lastUrl !== url) return;
      phase = "error";
      errorText = errorMessage(error);
    }
    render();
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
    };
  }

  async function startDownload(trigger?: HTMLButtonElement) {
    if (!result) return;
    const items =
      result.kind === "playlist"
        ? result.entries
            .filter((_, i) => selected.has(i))
            .map((e) => ({ url: e.url, title: e.title, thumbnail: e.thumbnail, uploader: e.uploader, duration: e.duration }))
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
      const jobs = await api.enqueue(items, currentOptions());
      jobs.forEach((job) => store.upsertJob(job));
      if (trigger) {
        trigger.classList.add("confirmed");
        replace(trigger, icon("check", 16), h("span", { text: jobs.length === 1 ? "Added" : `${jobs.length} Added` }));
        await new Promise((resolve) => setTimeout(resolve, 650));
      }
      toast(jobs.length === 1 ? "Added to the queue." : `${jobs.length} videos added to the queue.`, {
        action: { label: "View Queue", run: () => store.go("queue") },
      });
      input.value = "";
      lastUrl = "";
      result = null;
      phase = "idle";
      render();
    } catch (error) {
      toast(errorMessage(error), { tone: "error" });
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
    return h(
      "section",
      { class: "preview glass", attrs: { "aria-label": "Link details" } },
      thumbnailEl(art, "preview-thumb"),
      h(
        "div",
        { class: "preview-info" },
        h("h2", { class: "preview-title", text: r.title }),
        h("p", { class: "preview-meta", text: meta.join("  ·  ") }),
      ),
    );
  }

  function renderEntries(r: ProbeResult): HTMLElement {
    const countLabel = h("span", { class: "entries-count" });
    const list = h("div", { class: "entries", attrs: { role: "group", "aria-label": "Videos in this playlist" } });
    const downloadBtn: HTMLButtonElement = button({ icon: "download", variant: "primary", onClick: () => void startDownload(downloadBtn) });
    const paintCount = () => {
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
      h("div", { class: "entries-head" }, sectionTitle("Videos"), countLabel, toggleAll),
      h("div", { class: "glass group" }, list),
      h("div", { class: "actions" }, h("span", { class: "save-to", text: `Saves to ${folderName(store.settings.outputDir)}` }), downloadBtn),
    );
  }

  function renderOptions(r: ProbeResult): HTMLElement {
    const qualities = qualityChoices(r.heights);
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
              (value) => (height = value === 0 ? null : value),
            ),
            qualities.length === 0 ? "Quality options appear once the site reports them." : "Saved as MP4 when possible.",
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
            rows.push(row("Embed in video", toggle("Embed subtitles in the video", embedSubs, (v) => (embedSubs = v)), "Off saves them as separate .srt files."));
          }
        }
      } else {
        rows.push(
          row(
            "Format",
            chipRadio<string>("Audio format", AUDIO_FORMATS, audioFormat, (value) => (audioFormat = value)),
            "Original keeps the site's own audio without re-encoding.",
          ),
        );
      }
      replace(body, group(...rows));
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
      wrap.append(
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
            thumbnailEl(item.thumbnailLocal ? convertFileSrc(item.thumbnailLocal) : item.thumbnail, "recent-thumb"),
            h("div", { class: "recent-text" }, h("span", { class: "row-title", text: item.title }), h("span", { class: "row-hint", text: [item.quality, item.uploader].filter(Boolean).join("  ·  ") })),
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

  function render() {
    goButton.disabled = phase === "loading";
    switch (phase) {
      case "loading":
        replace(stage, h("div", { class: "loading", attrs: { role: "status" } }, spinner(), h("span", { text: "Reading the link…" })));
        break;
      case "error":
        replace(
          stage,
          h(
            "div",
            { class: "notice notice-error", attrs: { role: "alert" } },
            icon("alert", 18),
            h("div", { class: "notice-text" }, h("strong", { text: "Couldn't read that link" }), h("span", { text: errorText })),
            button({ label: "Try Again", variant: "secondary", size: "small", onClick: () => void submit() }),
          ),
        );
        break;
      case "result":
        replace(stage, renderResult(result!));
        stage.firstElementChild?.classList.add("settle");
        break;
      default:
        replace(
          stage,
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
  }

  const onPaste = (event: ClipboardEvent) => {
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
    const text = event.clipboardData?.getData("text")?.trim() ?? "";
    if (looksLikeUrl(text)) {
      input.value = text;
      void submit();
    }
  };
  document.addEventListener("paste", onPaste);
  const offHistory = store.on("history", () => phase === "idle" && render());

  render();
  input.focus();

  return () => {
    document.removeEventListener("paste", onPaste);
    offTools.forEach((off) => off());
    offHistory();
    lastUrl = "";
  };
};
