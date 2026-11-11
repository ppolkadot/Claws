import { h } from "./dom.js";
import { icon, type IconName } from "./icons.js";
import { canAnimate } from "./motion.js";

interface ButtonOptions {
  label?: string;
  icon?: IconName;
  variant?: "primary" | "secondary" | "plain" | "danger";
  title?: string;
  onClick?: (event: MouseEvent) => void;
  size?: "regular" | "small";
}

export function button(options: ButtonOptions): HTMLButtonElement {
  const { label, icon: iconName, variant = "secondary", size = "regular" } = options;
  const el = h(
    "button",
    {
      class: `btn btn-${variant} btn-${size}${label ? "" : " btn-icon"}`,
      attrs: {
        type: "button",
        title: options.title,
        "aria-label": label ? undefined : options.title,
      },
      on: options.onClick ? { click: options.onClick } : undefined,
    },
    iconName ? icon(iconName, size === "small" ? 14 : 16) : null,
    label ? h("span", { text: label }) : null,
  );
  return el;
}

export interface Segment<T extends string> {
  value: T;
  label: string;
  icon?: IconName;
  /** Shown but not choosable; `title` says why (also read out by screen readers). */
  disabled?: boolean;
  title?: string;
}

export interface SegmentedHandle<T extends string> {
  el: HTMLElement;
  set(value: T): void;
}

export function segmented<T extends string>(
  label: string,
  segments: Segment<T>[],
  value: T,
  onChange: (value: T) => void,
): SegmentedHandle<T> {
  const thumb = h("span", { class: "seg-thumb", attrs: { "aria-hidden": "true" } });
  const buttons = segments.map((segment, index) =>
    h(
      "button",
      {
        class: "seg-item",
        attrs: {
          type: "button",
          role: "radio",
          "data-index": index,
          title: segment.title,
          // aria-disabled, not disabled: it stays discoverable and explains itself.
          "aria-disabled": segment.disabled ? "true" : undefined,
        },
        on: {
          click: () => {
            if (segment.disabled) return;
            select(segment.value);
            onChange(segment.value);
          },
        },
      },
      segment.icon ? icon(segment.icon, 14) : null,
      h("span", { text: segment.label }),
    ),
  );
  const el = h(
    "div",
    {
      class: "segmented",
      attrs: { role: "radiogroup", "aria-label": label },
      style: { "--n": String(segments.length), "--i": "0" },
    },
    thumb,
    ...buttons,
  );

  function select(next: T) {
    const index = Math.max(0, segments.findIndex((s) => s.value === next));
    el.style.setProperty("--i", String(index));
    buttons.forEach((b, i) => {
      b.setAttribute("aria-checked", String(i === index));
      b.tabIndex = i === index ? 0 : -1;
    });
  }
  // Arrow keys move the choice (and the thumb) to the next segment that can be chosen.
  el.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const current = buttons.findIndex((b) => b.getAttribute("aria-checked") === "true");
    for (let i = 1; i < buttons.length; i++) {
      const next = (current + step * i + buttons.length * i) % buttons.length;
      if (segments[next].disabled) continue;
      buttons[next].focus();
      buttons[next].click();
      return;
    }
  });
  select(value);
  return { el, set: select };
}

export interface ChipOption<T extends string | number> {
  value: T;
  label: string;
  hint?: string;
}

/**
 * A single-choice row of pill buttons (radio semantics). Like a native radio group it is
 * one Tab stop: the checked chip (or the first, when nothing is checked) takes Tab, and
 * the arrow keys, Home and End move the choice, focus included.
 */
export function chipRadio<T extends string | number>(
  label: string,
  options: ChipOption<T>[],
  value: T,
  onChange: (value: T) => void,
): HTMLElement {
  const group = h("div", { class: "chips", attrs: { role: "radiogroup", "aria-label": label } });
  const choose = (index: number) => {
    mark(options[index].value);
    onChange(options[index].value);
  };
  const chips = options.map((option, index) =>
    h(
      "button",
      {
        class: "chip",
        attrs: { type: "button", role: "radio", title: option.hint },
        on: { click: () => choose(index) },
      },
      option.label,
    ),
  );
  function mark(current: T) {
    const checked = options.findIndex((option) => option.value === current);
    chips.forEach((chip, i) => {
      chip.setAttribute("aria-checked", String(i === checked));
      chip.tabIndex = i === (checked >= 0 ? checked : 0) ? 0 : -1;
    });
  }
  group.addEventListener("keydown", (event) => {
    if (chips.length === 0 || event.altKey || event.ctrlKey || event.metaKey) return;
    const checked = chips.findIndex((chip) => chip.getAttribute("aria-checked") === "true");
    const focused = chips.indexOf(event.target as HTMLButtonElement);
    const from = focused >= 0 ? focused : checked;
    let next: number;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        next = from < 0 ? 0 : (from + 1) % chips.length;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        next = from < 0 ? chips.length - 1 : (from - 1 + chips.length) % chips.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = chips.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    if (next === checked) {
      chips[next].focus();
      return;
    }
    choose(next);
    chips[next].focus();
  });
  chips.forEach((c) => group.appendChild(c));
  mark(value);
  return group;
}

/** A multi-choice row of pill buttons (checkbox semantics). */
export function chipCheck(
  label: string,
  options: ChipOption<string>[],
  selected: Set<string>,
  onChange: (selected: Set<string>) => void,
): HTMLElement {
  const group = h("div", { class: "chips", attrs: { role: "group", "aria-label": label } });
  for (const option of options) {
    const chip = h(
      "button",
      {
        class: "chip",
        attrs: {
          type: "button",
          role: "checkbox",
          "aria-checked": String(selected.has(option.value)),
        },
        on: {
          click: () => {
            if (selected.has(option.value)) selected.delete(option.value);
            else selected.add(option.value);
            chip.setAttribute("aria-checked", String(selected.has(option.value)));
            onChange(selected);
          },
        },
      },
      option.label,
    );
    group.appendChild(chip);
  }
  return group;
}

export function toggle(label: string, value: boolean, onChange: (value: boolean) => void): HTMLElement {
  const el = h(
    "button",
    {
      class: "switch",
      attrs: { type: "button", role: "switch", "aria-label": label, "aria-checked": String(value) },
    },
    h("span", { class: "switch-knob" }),
  );
  el.addEventListener("click", () => {
    const next = el.getAttribute("aria-checked") !== "true";
    el.setAttribute("aria-checked", String(next));
    onChange(next);
  });
  return el;
}

export function slider(
  label: string,
  value: number,
  onInput: (value: number) => void,
  min = 0,
  max = 100,
): HTMLInputElement {
  const el = h("input", {
    class: "slider",
    attrs: { type: "range", min, max, step: 1, value, "aria-label": label },
  });
  const paint = () => {
    const pct = ((Number(el.value) - min) / (max - min)) * 100;
    el.style.setProperty("--slider-pct", `${pct}%`);
  };
  el.addEventListener("input", () => {
    paint();
    onInput(Number(el.value));
  });
  paint();
  return el;
}

export function stepper(
  label: string,
  value: number,
  min: number,
  max: number,
  onChange: (value: number) => void,
): HTMLElement {
  const readout = h("span", { class: "stepper-value", text: String(value) });
  const set = (next: number) => {
    const clamped = Math.min(max, Math.max(min, next));
    readout.textContent = String(clamped);
    onChange(clamped);
  };
  return h(
    "div",
    { class: "stepper", attrs: { role: "group", "aria-label": label } },
    button({ icon: "minus", title: `Decrease ${label.toLowerCase()}`, variant: "plain", size: "small", onClick: () => set(Number(readout.textContent) - 1) }),
    readout,
    button({ icon: "plus", title: `Increase ${label.toLowerCase()}`, variant: "plain", size: "small", onClick: () => set(Number(readout.textContent) + 1) }),
  );
}

let toastHost: HTMLElement | null = null;
/** The least time a toast stays after the pointer or the keyboard leaves it. */
const TOAST_LINGER_MS = 1500;

export function toast(message: string, options: { tone?: "info" | "error"; action?: { label: string; run: () => void } } = {}) {
  if (!toastHost) {
    toastHost = h("div", { class: "toasts", attrs: { role: "status", "aria-live": "polite" } });
    document.body.appendChild(toastHost);
  }
  const { tone = "info", action } = options;
  // An error is announced at once (role="alert"); other toasts wait their turn in the
  // polite live region around them.
  const item = h(
    "div",
    { class: `toast toast-${tone}`, attrs: { role: tone === "error" ? "alert" : undefined } },
    icon(tone === "error" ? "alert" : "check", 16),
    h("span", { class: "toast-text", text: message }),
    action
      ? h("button", {
          class: "toast-action",
          text: action.label,
          attrs: { type: "button" },
          on: {
            click: () => {
              action.run();
              dismiss();
            },
          },
        })
      : null,
  );
  let timer = 0;
  let remaining = tone === "error" ? 7000 : 4200;
  let since = 0;
  let gone = false;
  const dismiss = () => {
    if (gone) return;
    gone = true;
    window.clearTimeout(timer);
    item.classList.add("leaving");
    window.setTimeout(() => item.remove(), 260);
  };
  const run = () => {
    if (gone) return;
    window.clearTimeout(timer);
    since = performance.now();
    timer = window.setTimeout(dismiss, remaining);
  };
  // The countdown waits while the pointer is over the toast or the keyboard is in it, so a
  // long message can be read; afterwards it gets at least a moment more.
  const hold = () => {
    if (gone) return;
    window.clearTimeout(timer);
    remaining = Math.max(TOAST_LINGER_MS, remaining - (performance.now() - since));
  };
  item.addEventListener("mouseenter", hold);
  item.addEventListener("mouseleave", run);
  item.addEventListener("focusin", hold);
  item.addEventListener("focusout", run);
  toastHost.appendChild(item);
  run();
}

export interface SheetAction {
  label: string;
  value: string;
  variant?: "primary" | "secondary" | "danger";
}

/**
 * A modal sheet. Destructive choices need protected focus, so this one blurs
 * the whole window behind it. Resolves to the chosen action's value, or null.
 *
 * The first decision wins. Once it is made the sheet is closing: every action is
 * disabled, and a later click, Escape or close request changes nothing, so dismissing
 * the sheet can never be turned into a confirmation (or the reverse) during the
 * short departure animation.
 */
export function confirmSheet(title: string, message: string, actions: SheetAction[]): Promise<string | null> {
  return new Promise((resolve) => {
    const dialog = h("dialog", { class: "sheet", attrs: { "aria-labelledby": "sheet-title" } });
    let result: string | null = null;
    let closing = false;
    let settled = false;
    const decide = (value: string | null) => {
      if (closing) return;
      closing = true;
      result = value;
      for (const control of controls) control.disabled = true;
      dismiss();
    };
    // The safe choice takes focus, so Enter never confirms something destructive.
    const cancel = button({ label: "Cancel", variant: "plain", onClick: () => decide(null) });
    cancel.autofocus = true;
    const controls = [
      ...actions.map((action) =>
        button({
          label: action.label,
          variant: action.variant ?? "secondary",
          onClick: () => decide(action.value),
        }),
      ),
      cancel,
    ];
    const body = h(
      "div",
      { class: "sheet-body" },
      h("h2", { class: "sheet-title", text: title, attrs: { id: "sheet-title" } }),
      h("p", { class: "sheet-message", text: message }),
      h("div", { class: "sheet-actions" }, ...controls),
    );
    dialog.appendChild(body);
    dialog.addEventListener("close", () => {
      dialog.remove();
      if (settled) return;
      settled = true;
      resolve(result);
    });
    function dismiss() {
      // No departure with reduced motion or in a hidden window: the sheet just closes.
      if (!canAnimate()) return dialog.close();
      const out = dialog.animate([{ opacity: 1, transform: "none" }, { opacity: 0, transform: "scale(0.96)" }], {
        duration: 160,
        easing: "cubic-bezier(0.4, 0, 1, 1)",
      });
      // A cancelled animation never finishes, and a paused one (a window hidden mid-way)
      // may never end; neither may leave the sheet stuck open with its actions disabled.
      const fallback = window.setTimeout(() => dialog.close(), 400);
      dialog.addEventListener("close", () => window.clearTimeout(fallback));
      out.onfinish = () => dialog.close();
      out.oncancel = () => dialog.close();
    }
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      decide(null);
    });
    document.body.appendChild(dialog);
    dialog.showModal();
  });
}
