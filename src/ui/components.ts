import { h } from "./dom";
import { icon, type IconName } from "./icons";
import { canAnimate } from "./motion";

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

/** A single-choice row of pill buttons (radio semantics). */
export function chipRadio<T extends string | number>(
  label: string,
  options: ChipOption<T>[],
  value: T,
  onChange: (value: T) => void,
): HTMLElement {
  const group = h("div", { class: "chips", attrs: { role: "radiogroup", "aria-label": label } });
  const chips = options.map((option) =>
    h(
      "button",
      {
        class: "chip",
        attrs: { type: "button", role: "radio", title: option.hint },
        on: {
          click: () => {
            mark(option.value);
            onChange(option.value);
          },
        },
      },
      option.label,
    ),
  );
  function mark(current: T) {
    chips.forEach((chip, i) => chip.setAttribute("aria-checked", String(options[i].value === current)));
  }
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

export function toast(message: string, options: { tone?: "info" | "error"; action?: { label: string; run: () => void } } = {}) {
  if (!toastHost) {
    toastHost = h("div", { class: "toasts", attrs: { role: "status", "aria-live": "polite" } });
    document.body.appendChild(toastHost);
  }
  const { tone = "info", action } = options;
  const item = h(
    "div",
    { class: `toast toast-${tone}` },
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
  const dismiss = () => {
    item.classList.add("leaving");
    window.setTimeout(() => item.remove(), 260);
  };
  toastHost.appendChild(item);
  window.setTimeout(dismiss, tone === "error" ? 7000 : 4200);
}

export interface SheetAction {
  label: string;
  value: string;
  variant?: "primary" | "secondary" | "danger";
}

/**
 * A modal sheet. Destructive choices need protected focus, so this one blurs
 * the whole window behind it. Resolves to the chosen action's value, or null.
 */
export function confirmSheet(title: string, message: string, actions: SheetAction[]): Promise<string | null> {
  return new Promise((resolve) => {
    const dialog = h("dialog", { class: "sheet", attrs: { "aria-labelledby": "sheet-title" } });
    let result: string | null = null;
    // The safe choice takes focus, so Enter never confirms something destructive.
    const cancel = button({ label: "Cancel", variant: "plain", onClick: () => dialog.close() });
    cancel.autofocus = true;
    const body = h(
      "div",
      { class: "sheet-body" },
      h("h2", { class: "sheet-title", text: title, attrs: { id: "sheet-title" } }),
      h("p", { class: "sheet-message", text: message }),
      h(
        "div",
        { class: "sheet-actions" },
        ...actions.map((action) =>
          button({
            label: action.label,
            variant: action.variant ?? "secondary",
            onClick: () => {
              result = action.value;
              dialog.close();
            },
          }),
        ),
        cancel,
      ),
    );
    dialog.appendChild(body);
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(result);
    });
    const close = dialog.close.bind(dialog);
    dialog.close = () => {
      // No departure with reduced motion or in a hidden window: the sheet just closes.
      if (!canAnimate()) return close();
      const out = dialog.animate([{ opacity: 1, transform: "none" }, { opacity: 0, transform: "scale(0.96)" }], {
        duration: 160,
        easing: "cubic-bezier(0.4, 0, 1, 1)",
      });
      out.onfinish = () => close();
    };
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      dialog.close();
    });
    document.body.appendChild(dialog);
    dialog.showModal();
  });
}
