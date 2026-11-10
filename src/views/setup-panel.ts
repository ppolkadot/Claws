import { api, type ToolsStatus } from "../api";
import { SETUP_STEPS, missingTools, stepState, toolReady, type StepState, type ToolName } from "../setup";
import { store } from "../store";
import { button } from "../ui/components";
import { clear, h, replace } from "../ui/dom";
import { progressBar, setProgress } from "../ui/layout";

const INSTALL: Record<ToolName, () => Promise<ToolsStatus>> = {
  ytdlp: api.installYtdlp,
  ffmpeg: api.installFfmpeg,
  deno: api.installDeno,
};

const STATE_TEXT: Record<StepState, string> = {
  ready: "Ready",
  installing: "Installing…",
  waiting: "Waiting",
  error: "Could not install",
  missing: "Not installed",
};

/**
 * The first-run setup panel: one button installs whatever yt-dlp, ffmpeg and the
 * JavaScript runtime are missing, with a row per tool. Nothing is downloaded until the
 * button is pressed, and the panel goes away once all three are ready.
 */
export function mountSetupPanel(slot: HTMLElement): () => void {
  /** Tools this panel asked for that have not finished yet, so their rows read "Waiting". */
  const queued = new Set<ToolName>();

  const setUpEverything = () => {
    for (const tool of missingTools(store.tools)) {
      if (store.toolBusy[tool]) continue;
      queued.add(tool);
      void store.runTool(tool, INSTALL[tool]).finally(() => {
        queued.delete(tool);
        paint();
      });
    }
    paint();
  };

  const paint = () => {
    if (!store.toolsKnown) return clear(slot);
    const missing = missingTools(store.tools);
    const working = SETUP_STEPS.some((step) => store.toolBusy[step.tool]);
    // yt-dlp is required, so its absence cannot be dismissed; the other two can.
    const dismissed = store.setupDismissed && !missing.includes("ytdlp") && !working;
    if ((missing.length === 0 && !working) || dismissed) return clear(slot);

    const failed = missing.some((tool) => store.toolError[tool]);
    const rows = SETUP_STEPS.map((step) => {
      const busy = Boolean(store.toolBusy[step.tool]);
      const error = store.toolError[step.tool];
      const state = stepState(toolReady(store.tools, step.tool), busy, queued.has(step.tool), error);
      const progress = store.toolProgress[step.tool];
      const fraction = progress && progress.total > 0 ? progress.received / progress.total : null;
      const bar = progressBar(fraction);
      setProgress(bar, fraction);
      return h(
        "li",
        { class: "setup-step", attrs: { "data-state": state } },
        h("span", { class: "tool-dot", attrs: { "data-state": state === "ready" ? "ok" : state === "error" ? "error" : "missing", "aria-hidden": "true" } }),
        h(
          "div",
          { class: "setup-step-text" },
          h("strong", { text: step.name }),
          h("span", { text: state === "error" && error ? error : step.purpose }),
          state === "installing" ? bar : null,
        ),
        h("span", { class: "setup-step-state", text: state === "installing" && progress?.stage === "extract" ? "Unpacking…" : STATE_TEXT[state] }),
      );
    });

    const start = button({
      label: working ? "Setting Up…" : failed ? "Try Again" : "Set Up Everything",
      icon: failed && !working ? "retry" : "download",
      variant: "primary",
      onClick: setUpEverything,
    });
    start.disabled = working;

    replace(
      slot,
      h(
        "section",
        { class: `setup-panel glass${failed ? " banner-error" : ""}`, attrs: { role: "status", "aria-label": "Set up Claws" } },
        h(
          "div",
          { class: "banner-text" },
          h("strong", { text: failed ? "Setup could not finish" : "Set up Claws" }),
          h("span", {
            text: missing.includes("ytdlp")
              ? "Claws needs these tools to download. They are fetched once from their projects' GitHub releases and checked before use."
              : "Some sites and formats need these tools. They are fetched once from their projects' GitHub releases and checked before use.",
          }),
        ),
        h("ul", { class: "setup-steps" }, ...rows),
        h(
          "div",
          { class: "setup-actions" },
          missing.includes("ytdlp") || working
            ? null
            : button({ label: "Not Now", variant: "plain", onClick: () => store.dismissSetup() }),
          start,
        ),
      ),
    );
  };

  const off = [store.on("tools", paint), store.on("toolProgress", paint)];
  paint();
  return () => off.forEach((stop) => stop());
}
