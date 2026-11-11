import "./styles/fonts.css";
import "./styles/base.css";
import "./styles/components.css";
import "./styles/views.css";

import { ProgressBarStatus, getCurrentWindow } from "@tauri-apps/api/window";

import { api, onEvent, errorMessage, type Job, type JobEvent, type JobRemoved, type JobsAdded, type ToolProgress } from "./api";
import { store } from "./store";
import { StartupEvents, StatusTransitions } from "./queue-events";
import { sameTaskbarState, taskbarState, type TaskbarState } from "./queue-summary";
import { h } from "./ui/dom";
import { toast } from "./ui/components";
import { installSprings } from "./ui/motion";
import { looksLikeLinkList, looksLikeUrl, mountDownload } from "./views/download";
import { mountLibrary } from "./views/library";
import { mountQueue } from "./views/queue";
import { mountSettings } from "./views/settings";
import { buildShell } from "./views/shell";

const root = document.querySelector<HTMLDivElement>("#app")!;

function applyAppearance() {
  const { theme, glassIntensity, backdrop } = store.settings;
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  const el = document.documentElement;
  el.dataset.theme = dark ? "dark" : "light";
  el.dataset.backdrop = backdrop;
  el.style.setProperty("--glass-i", String(glassIntensity / 100));
}

async function start() {
  installSprings();
  // A plain browser has no backend; dev builds substitute sample data.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installMock } = await import("./mock");
    installMock();
  }

  let notices: string[] = [];
  try {
    // Recovery can finish while bootstrap is in flight. Subscribe first and replay
    // its events after loading the snapshot so no completion/removal is lost.
    const startup = new StartupEvents();
    await onEvent<JobEvent<Job>>("job-update", (event) => startup.accept(() => store.updateJob(event)));
    await onEvent<JobsAdded<Job>>("jobs-added", (batch) => startup.accept(() => store.addJobs(batch)));
    await onEvent<JobRemoved>("job-removed", (event) => startup.accept(() => store.removeJob(event)));
    await onEvent<null>("history-changed", () => startup.accept(() => void store.refreshHistory()));
    notices = await store.load();
    startup.flush();
  } catch (error) {
    root.replaceChildren(
      h("div", { class: "fatal glass" }, h("h1", { text: "Claws could not start" }), h("p", { text: errorMessage(error) })),
    );
    void api.appReady().catch(() => undefined);
    return;
  }

  applyAppearance();

  void store.loadTools().then(() => {
    // Move installs from the slower single-file yt-dlp build to the folder build.
    const engine = store.tools.ytdlp;
    // Quiet: if yt-dlp is busy, it tries again next launch.
    if (engine?.managed && !engine.fastStart && store.activeCount() === 0) {
      void store.runTool("ytdlp", api.installYtdlp, { quiet: true });
    }
  });
  store.on("settings", applyAppearance);
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyAppearance);

  buildShell(root, {
    download: { title: "Download", mount: mountDownload },
    queue: { title: "Queue", mount: mountQueue },
    library: { title: "Library", mount: mountLibrary },
    settings: { title: "Settings", mount: mountSettings },
  });

  await onEvent<ToolProgress>("tool-progress", (progress) => store.setToolProgress(progress));
  await onEvent<string>("notice", (message) => toast(message, { tone: "error" }));
  notices.forEach((message) => toast(message, { tone: "error" }));

  // Ctrl+V with a link anywhere in the window (outside a text field) looks it up,
  // whichever view is open. A list of links is put in the link field to look up.
  document.addEventListener("paste", (event) => {
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) return;
    const text = event.clipboardData?.getData("text")?.trim() ?? "";
    if (looksLikeUrl(text) || looksLikeLinkList(text)) {
      event.preventDefault();
      store.pasteLink(text);
    }
  });

  if ("__TAURI_INTERNALS__" in window) {
    const win = getCurrentWindow();
    void win.onCloseRequested(async (event) => {
      // Save a pending settings edit first. Bounded and never throws, so closing always works.
      try {
        await Promise.race([store.flushSettings(), new Promise((resolve) => setTimeout(resolve, 1500))]);
      } catch {
        // Closing matters more than the last edit.
      }
      // Like a Mac app, closing does not quit; Claws stays in the notification area.
      if (store.settings.keepRunning) {
        event.preventDefault();
        void win.hide();
      }
    });
  }

  // Quick Open looks up a link on the clipboard, but not the same one twice in a row.
  let lastQuickLink = "";
  await onEvent<null>("quick-open", () => {
    void (async () => {
      let text = "";
      try {
        text = (await navigator.clipboard.readText()).trim();
      } catch {
      }
      if (looksLikeUrl(text) && text !== lastQuickLink) {
        lastQuickLink = text;
        store.pasteLink(text);
      } else {
        store.go("download");
        store.emit("focusLink");
      }
    })();
  });

  // The taskbar button follows the running downloads, at most twice a second, and only
  // when what it shows would change.
  if ("__TAURI_INTERNALS__" in window) {
    let shown: TaskbarState = { status: "none" };
    let taskbarTimer = 0;
    const paintTaskbar = () => {
      taskbarTimer = 0;
      const next = taskbarState(store.jobs.values());
      if (sameTaskbarState(shown, next)) return;
      shown = next;
      const status = next.status === "none" ? ProgressBarStatus.None : next.status === "indeterminate" ? ProgressBarStatus.Indeterminate : ProgressBarStatus.Normal;
      void getCurrentWindow()
        .setProgressBar({ status, progress: next.status === "normal" ? next.progress : undefined })
        .catch(() => {
          // Cosmetic: the Queue still shows the progress.
        });
    };
    const scheduleTaskbar = () => {
      if (!taskbarTimer) taskbarTimer = window.setTimeout(paintTaskbar, 500);
    };
    store.on("job", scheduleTaskbar);
    store.on("jobs", scheduleTaskbar);
    scheduleTaskbar();
  }

  const transitions = new StatusTransitions();
  transitions.seed(store.jobList());
  store.on("jobs", () => transitions.prune(store.jobList().map((job) => job.id)));
  store.on("job", (detail) => {
    const job = detail as Job;
    const notice = transitions.observe(job);
    if (notice === "completed") {
      if (store.route !== "queue") {
        toast(`Finished: ${job.title}`, { action: { label: "Show", run: () => void api.revealDownload(job.id).catch(() => store.go("library")) } });
      }
    } else if (notice === "failed" && store.route !== "queue") {
      toast(`Couldn't download “${job.title}”.`, { tone: "error", action: { label: "Details", run: () => store.go("queue") } });
    }
  });

  // Show the window only after the font and first frame, so it never flashes blank or
  // in the fallback face.
  const fontsIn = Promise.all([400, 500, 600, 700].map((w) => document.fonts.load(`${w} 13px "Claws Polaris"`)));
  await Promise.race([fontsIn, new Promise((resolve) => setTimeout(resolve, 800))]).catch(() => undefined);
  requestAnimationFrame(() => requestAnimationFrame(() => void api.appReady()));
}

void start();
