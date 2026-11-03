import { h } from "./dom";

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

/** `null` shows the indeterminate sheen. */
export function setProgress(bar: HTMLElement, fraction: number | null) {
  if (fraction === null) {
    bar.classList.add("indeterminate");
    bar.removeAttribute("aria-valuenow");
    return;
  }
  bar.classList.remove("indeterminate");
  const clamped = Math.min(1, Math.max(0, fraction));
  bar.style.setProperty("--p", String(clamped));
  bar.setAttribute("role", "progressbar");
  bar.setAttribute("aria-valuemin", "0");
  bar.setAttribute("aria-valuemax", "100");
  bar.setAttribute("aria-valuenow", String(Math.round(clamped * 100)));
}
