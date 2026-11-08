import { h } from "./dom.js";

type Control = Node | string;

/** One row of an inset-grouped list: label and optional hint on the left, control on the right. */
export function row(label: string, control: Control | null, hint?: string | Node): HTMLElement {
  return h(
    "div",
    { class: "row" },
    h(
      "div",
      { class: "row-label" },
      h("span", { class: "row-title", text: label }),
      hint ? h("span", { class: "row-hint" }, hint) : null,
    ),
    control ? h("div", { class: "row-control" }, control) : null,
  );
}

/** A glass panel that stacks rows separated by hairlines. */
export function group(...rows: (Node | null)[]): HTMLElement {
  return h("div", { class: "group glass" }, ...rows);
}

export function sectionTitle(text: string): HTMLElement {
  return h("h2", { class: "section-title", text });
}

export function spinner(): HTMLElement {
  const el = h("span", { class: "spinner", attrs: { role: "progressbar", "aria-label": "Loading" } });
  for (let i = 0; i < 8; i++) {
    el.appendChild(
      h("i", { style: { transform: `rotate(${i * 45}deg)`, "animation-delay": `${(i - 8) * 0.1}s` } }),
    );
  }
  return el;
}

export function progressBar(fraction: number | null): HTMLElement {
  const bar = h("div", { class: "bar" }, h("span", { class: "bar-fill" }));
  setProgress(bar, fraction);
  return bar;
}

// Bars snapped since the last frame. Their transition stays off for the frame that draws
// the new value and comes back on the frame after; one shared pair of frames for all of
// them, so a burst of snaps costs no more than one.
let snapped: Set<HTMLElement> | null = null;

function snapThisFrame(bar: HTMLElement) {
  bar.classList.add("bar-snap");
  if (!snapped) {
    const batch = (snapped = new Set());
    requestAnimationFrame(() => {
      snapped = null;
      requestAnimationFrame(() => {
        // A bar snapped again since then keeps its class until its own frame has drawn.
        for (const b of batch) if (!snapped?.has(b)) b.classList.remove("bar-snap");
      });
    });
  }
  snapped.add(bar);
}

/**
 * `null` shows the indeterminate sheen. A value is clamped to 0–100%. "smooth" lets the
 * fill glide forwards to it (a CSS transform transition); "snap" shows it at once, for a
 * first paint, a status change or a retry. Whoever calls, a value lower than the one shown
 * (a retry, the next tool's download on the same bar), leaving the sheen and a hidden
 * window always snap, so no bar ever slides backwards. Reads only the bar's own inline
 * `--p`, never layout.
 */
export function setProgress(bar: HTMLElement, fraction: number | null, motion: "smooth" | "snap" = "smooth") {
  if (fraction === null) {
    bar.classList.add("indeterminate");
    bar.removeAttribute("aria-valuenow");
    return;
  }
  const clamped = Math.min(1, Math.max(0, fraction));
  const shown = Number.parseFloat(bar.style.getPropertyValue("--p"));
  const hidden = typeof document !== "undefined" && document.hidden;
  // The sheen never morphs into the fill: leaving it always snaps.
  if (motion === "snap" || hidden || clamped < shown || bar.classList.contains("indeterminate")) snapThisFrame(bar);
  bar.classList.remove("indeterminate");
  bar.style.setProperty("--p", String(clamped));
  bar.setAttribute("role", "progressbar");
  bar.setAttribute("aria-valuemin", "0");
  bar.setAttribute("aria-valuemax", "100");
  bar.setAttribute("aria-valuenow", String(Math.round(clamped * 100)));
}
