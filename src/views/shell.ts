import { getCurrentWindow } from "@tauri-apps/api/window";

import { api } from "../api";
import { store, type Route } from "../store";
import { h, clear } from "../ui/dom";
import { icon, type IconName } from "../ui/icons";
import { progressBar, setProgress } from "../ui/layout";
import { pop } from "../ui/motion";

export interface ViewContext {
  setActions(nodes: Node[]): void;
}
export type MountView = (container: HTMLElement, ctx: ViewContext) => () => void;

const NAV: { route: Route; label: string; icon: IconName }[] = [
  { route: "download", label: "Download", icon: "download" },
  { route: "queue", label: "Queue", icon: "list" },
  { route: "library", label: "Library", icon: "grid" },
  { route: "settings", label: "Settings", icon: "sliders" },
];

const SIDEBAR_KEY = "claws.sidebar";

function readSidebarOpen(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_KEY) !== "closed";
  } catch {
    return true;
  }
}

function saveSidebarOpen(open: boolean) {
  try {
    localStorage.setItem(SIDEBAR_KEY, open ? "open" : "closed");
  } catch {
    // Not remembering the sidebar is harmless.
  }
}

function trafficLights(): HTMLElement {
  const win = getCurrentWindow();
  const light = (cls: string, label: string, glyph: IconName, run: () => void) =>
    h(
      "button",
      { class: `light ${cls}`, attrs: { type: "button", "aria-label": label, title: label }, on: { click: run } },
      icon(glyph, 8),
    );
  return h(
    "div",
    { class: "traffic" },
    light("light-close", "Close", "x", () => void win.close()),
    light("light-min", "Minimize", "minus", () => void win.minimize()),
    light("light-max", "Zoom", "plus", () => void win.toggleMaximize()),
  );
}

export function buildShell(root: HTMLElement, views: Record<Route, { title: string; mount: MountView }>) {
  const navButtons = new Map<Route, HTMLButtonElement>();
  const badge = h("span", { class: "nav-badge", attrs: { "aria-live": "polite" } });
  const indicator = h("span", { class: "nav-indicator", attrs: { "aria-hidden": "true" } });
  const nav = h("nav", { class: "nav", attrs: { "aria-label": "Sections" } }, indicator);
  for (const item of NAV) {
    const button = h(
      "button",
      { class: "nav-item", attrs: { type: "button", title: `${item.label} (Ctrl+${NAV.indexOf(item) + 1})` }, on: { click: () => store.go(item.route) } },
      icon(item.icon, 17),
      h("span", { class: "nav-label", text: item.label }),
      item.route === "queue" ? badge : null,
    );
    navButtons.set(item.route, button);
    nav.appendChild(button);
  }

  const toolDot = h("span", { class: "tool-dot" });
  const toolText = h("span", { class: "tool-text" });
  const toolBar = progressBar(0);
  const toolChip = h(
    "button",
    { class: "tool-chip", attrs: { type: "button", title: "Open tool settings" }, on: { click: () => store.go("settings") } },
    h("span", { class: "tool-line" }, toolDot, toolText),
    toolBar,
  );

  const sidebarPanel = h(
    "div",
    { class: "sidebar-panel" },
    h("div", { class: "sidebar-head", attrs: { "data-tauri-drag-region": true } }),
    nav,
    h("div", { class: "sidebar-foot" }, toolChip),
  );
  const sidebar = h("aside", { class: "sidebar" }, sidebarPanel);

  const toolbarTitle = h("div", { class: "toolbar-title", attrs: { "data-tauri-drag-region": true, "aria-hidden": "true" } });
  const actions = h("div", { class: "toolbar-actions" });
  const toolbar = h("header", { class: "toolbar", attrs: { "data-tauri-drag-region": true } }, toolbarTitle, actions);
  const largeTitle = h("h1", { class: "large-title" });
  const view = h("div", { class: "view" });
  const page = h("div", { class: "page" }, largeTitle, view);
  const scroller = h("div", { class: "scroll" }, page);
  const content = h("main", { class: "content" }, toolbar, scroller);

  const app = h("div", { class: "app" }, sidebar, content);

  const toggle = h(
    "button",
    { class: "btn btn-glass", attrs: { type: "button" }, on: { click: () => setSidebar(app.dataset.sidebar !== "open") } },
    icon("sidebar", 16),
  );
  const chrome = h("div", { class: "chrome" }, trafficLights(), toggle);

  root.append(chrome, app);

  function setSidebar(open: boolean, persist = true) {
    app.dataset.sidebar = open ? "open" : "closed";
    // Hidden content must not be reachable by keyboard while it is off screen.
    sidebar.toggleAttribute("inert", !open);
    const label = `${open ? "Hide" : "Show"} Sidebar (Ctrl+Alt+S)`;
    toggle.setAttribute("aria-label", label);
    toggle.title = label;
    toggle.setAttribute("aria-expanded", String(open));
    if (persist) saveSidebarOpen(open);
  }
  setSidebar(readSidebarOpen(), false);
  document.addEventListener("keydown", (event) => {
    if (event.ctrlKey && event.altKey && event.key.toLowerCase() === "s") {
      event.preventDefault();
      setSidebar(app.dataset.sidebar !== "open");
    } else if (event.ctrlKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "q") {
      // Quit, as on a Mac: closing only hides Claws.
      event.preventDefault();
      void Promise.race([store.flushSettings(), new Promise((resolve) => setTimeout(resolve, 1500))])
        .catch(() => undefined)
        .then(() => api.quit());
    } else if (event.ctrlKey && !event.altKey && !event.shiftKey && /^[1-4]$/.test(event.key)) {
      event.preventDefault();
      store.go(NAV[Number(event.key) - 1].route);
    }
  });

  const placeIndicator = () => {
    const active = navButtons.get(store.route);
    if (!active) return;
    indicator.style.setProperty("--y", `${active.offsetTop}px`);
  };

  const observer = new IntersectionObserver(
    ([entry]) => content.classList.toggle("scrolled", !entry.isIntersecting),
    { root: scroller, rootMargin: "-52px 0px 0px 0px", threshold: 0 },
  );
  observer.observe(largeTitle);

  let cleanup: (() => void) | null = null;
  let first = true;
  const swap = () => {
    cleanup?.();
    clear(actions);
    clear(view);
    const current = views[store.route];
    largeTitle.textContent = current.title;
    toolbarTitle.textContent = current.title;
    document.title = `${current.title} — Claws`;
    for (const [route, button] of navButtons) {
      if (route === store.route) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    }
    cleanup = current.mount(view, {
      setActions: (nodes) => {
        clear(actions);
        nodes.forEach((n) => actions.appendChild(n));
      },
    });
    scroller.scrollTop = 0;
  };
  const renderRoute = () => {
    placeIndicator(); // moves on its own spring, outside the snapshot
    const animate = !first && !matchMedia("(prefers-reduced-motion: reduce)").matches;
    first = false;
    if (animate && "startViewTransition" in document) {
      (document as Document & { startViewTransition(run: () => void): unknown }).startViewTransition(swap);
    } else {
      swap();
    }
  };
  store.on("route", renderRoute);
  renderRoute();
  requestAnimationFrame(placeIndicator);

  let lastCount = 0;
  const paintBadge = () => {
    const count = store.activeCount();
    badge.textContent = count > 0 ? String(count) : "";
    if (count > lastCount) pop(badge);
    lastCount = count;
  };
  store.on("jobStatus", paintBadge);
  paintBadge();

  const paintTools = () => {
    const busy = store.toolBusy.ytdlp || store.toolBusy.ffmpeg;
    const tool = store.toolBusy.ytdlp ? "ytdlp" : "ffmpeg";
    const progress = store.toolProgress[tool];
    toolChip.classList.toggle("busy", Boolean(busy));
    toolDot.dataset.state = !store.toolsKnown ? "" : store.toolError.ytdlp ? "error" : store.tools.ytdlp ? "ok" : "missing";
    if (busy) {
      const pct = progress && progress.total > 0 ? progress.received / progress.total : null;
      const name = tool === "ytdlp" ? "yt-dlp" : "ffmpeg";
      toolText.textContent =
        progress?.stage === "extract"
          ? `Unpacking ${name}…`
          : `Setting up ${name}${pct !== null ? ` ${Math.round(pct * 100)}%` : "…"}`;
      setProgress(toolBar, pct);
    } else if (!store.toolsKnown) {
      toolText.textContent = "Checking yt-dlp…";
    } else if (store.toolError.ytdlp) {
      toolText.textContent = "yt-dlp setup failed";
    } else if (store.tools.ytdlp) {
      toolText.textContent = `yt-dlp ${store.tools.ytdlp.version}`;
    } else {
      toolText.textContent = "yt-dlp not installed";
    }
  };
  store.on("tools", paintTools);
  store.on("toolProgress", paintTools);
  paintTools();

  void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    document.documentElement.classList.toggle("inactive", !focused);
  });
}
