import "./styles/fonts.css";
import "./styles/base.css";
import "./styles/components.css";
import "./styles/views.css";

import { api, onEvent, errorMessage, type Job, type ToolProgress } from "./api";
import { store } from "./store";
import { h } from "./ui/dom";
import { toast } from "./ui/components";
import { installSprings } from "./ui/motion";
import { mountDownload } from "./views/download";
import { mountLibrary } from "./views/library";
import { mountQueue } from "./views/queue";
import { mountSettings } from "./views/settings";
import { buildShell } from "./views/shell";

const root = document.querySelector<HTMLDivElement>("#app")!;

/** Pushes the appearance settings onto the document so they apply before any save round trip. */
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

  try {
    await store.load();
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
    if (engine?.managed && !engine.fastStart && store.activeCount() === 0) {
      void store.runTool("ytdlp", api.installYtdlp);
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

  await onEvent<Job>("job-update", (job) => store.upsertJob(job));
  await onEvent<string>("job-removed", (id) => store.removeJob(id));
  await onEvent<null>("history-changed", () => void store.refreshHistory());
  await onEvent<ToolProgress>("tool-progress", (progress) => store.setToolProgress(progress));

  const finished = new Set<string>();
  store.on("job", (detail) => {
    const job = detail as Job;
    if (job.status === "completed" && !finished.has(job.id)) {
      finished.add(job.id);
      if (store.route !== "queue") {
        toast(`Finished: ${job.title}`, { action: { label: "Show", run: () => void api.revealDownload(job.id).catch(() => store.go("library")) } });
      }
    } else if (job.status === "failed" && store.route !== "queue") {
      toast(`Couldn't download “${job.title}”.`, { tone: "error", action: { label: "Details", run: () => store.go("queue") } });
    }
  });

  // Show the window only after the font and first frame, so it never flashes blank or
  // in the fallback face.
  const fontsIn = Promise.all([400, 500, 600, 700].map((w) => document.fonts.load(`${w} 13px "Legacy UI Display"`)));
  await Promise.race([fontsIn, new Promise((resolve) => setTimeout(resolve, 800))]).catch(() => undefined);
  requestAnimationFrame(() => requestAnimationFrame(() => void api.appReady()));
}

void start();
