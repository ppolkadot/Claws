// CSS cannot run a spring, but `linear()` can replay one sampled ahead of time.

export interface Spring {
  /** Seconds the spring takes to cover most of the distance (SwiftUI's `response`). */
  response: number;
  /** 1 is critically damped (no overshoot); lower values overshoot and settle. */
  damping: number;
}

export const springs = {
  /** Layout changes: sidebar, panels resizing. No overshoot. */
  smooth: { response: 0.45, damping: 1 },
  /** Small controls: segmented thumb, toggles, selection. A hint of overshoot. */
  snappy: { response: 0.32, damping: 0.82 },
  /** Things that arrive: sheets, toasts, confirmations. Visible but brief bounce. */
  bouncy: { response: 0.42, damping: 0.68 },
} satisfies Record<string, Spring>;

/** Position of a spring released from 0 towards 1, at time `t` seconds. */
function position({ response, damping }: Spring, t: number): number {
  const omega = (2 * Math.PI) / response;
  if (damping >= 1) {
    return 1 - Math.exp(-omega * t) * (1 + omega * t);
  }
  const decay = damping * omega;
  const omegaD = omega * Math.sqrt(1 - damping * damping);
  return 1 - Math.exp(-decay * t) * (Math.cos(omegaD * t) + (decay / omegaD) * Math.sin(omegaD * t));
}

/** How long until the spring is visually at rest (within 0.1% and staying there). */
function settleTime(spring: Spring): number {
  const step = 1 / 240;
  let lastMoving = 0;
  for (let t = 0; t < 3; t += step) {
    if (Math.abs(1 - position(spring, t)) > 0.001) lastMoving = t;
  }
  return Math.min(3, lastMoving + step);
}

export interface SampledSpring {
  easing: string;
  /** Milliseconds. */
  duration: number;
}

export function sample(spring: Spring, points = 48): SampledSpring {
  const duration = settleTime(spring);
  const values: string[] = [];
  for (let i = 0; i <= points; i++) {
    const v = i === points ? 1 : position(spring, (i / points) * duration);
    values.push(String(Math.round(v * 10000) / 10000));
  }
  return { easing: `linear(${values.join(", ")})`, duration: Math.round(duration * 1000) };
}

const sampled = Object.fromEntries(
  Object.entries(springs).map(([name, spring]) => [name, sample(spring)]),
) as Record<keyof typeof springs, SampledSpring>;

export const reducedMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Publishes the springs as CSS variables: `--spring-<name>` and `--spring-<name>-d`. */
export function installSprings() {
  if (!CSS.supports("transition-timing-function", "linear(0, 1)")) return; // CSS keeps its fallbacks
  const root = document.documentElement.style;
  for (const [name, { easing, duration }] of Object.entries(sampled)) {
    root.setProperty(`--spring-${name}`, easing);
    root.setProperty(`--spring-${name}-d`, `${duration}ms`);
  }
}

export function spring(name: keyof typeof springs): SampledSpring {
  return reducedMotion() ? { easing: "linear", duration: 1 } : sampled[name];
}

/** Movement is worth starting: motion is allowed and someone can see the window. */
export const canAnimate = () => !reducedMotion() && !document.hidden;

// The one movement each element is running (arrival, departure or completion), so a newer
// one, a retry, a removal or the view closing can cancel it instead of stacking another.
const running = new WeakMap<Element, Animation>();

function own(el: Element, animation: Animation): Animation {
  running.set(el, animation);
  animation.addEventListener?.("finish", () => {
    if (running.get(el) === animation) running.delete(el);
  });
  return animation;
}

/** Cancels whatever movement these elements are running; they show their final state. */
export function cancelMotion(...els: Element[]) {
  for (const el of els) {
    running.get(el)?.cancel();
    running.delete(el);
  }
}

/**
 * Grows an element from zero height into place, as a new row arriving in a list. Pass
 * `height` when the caller has measured a batch of rows in one read. Returns the
 * animation, or null (no movement) with reduced motion or in a hidden window.
 */
export function expandIn(el: HTMLElement, height?: number): Animation | null {
  if (!canAnimate()) return null;
  const to = height ?? el.getBoundingClientRect().height;
  const { easing, duration } = spring("smooth");
  cancelMotion(el);
  return own(
    el,
    el.animate(
      [
        { height: "0px", opacity: 0, transform: "scale(0.98)" },
        { height: `${to}px`, opacity: 1, transform: "none" },
      ],
      { duration, easing },
    ),
  );
}

/**
 * Collapses an element to zero height, then removes it; at once with reduced motion or in
 * a hidden window. A row still arriving has its arrival cancelled first, so the two never
 * stack. Pass `height` when the caller has measured a batch of rows in one read. Returns
 * the collapse, so a caller leaving the view can cancel it (`cancelMotion`) and remove the
 * element itself; null when the element went at once.
 */
export function collapseOut(el: HTMLElement, height?: number): Animation | null {
  if (!canAnimate()) {
    cancelMotion(el);
    el.remove();
    return null;
  }
  const from = height ?? el.getBoundingClientRect().height;
  cancelMotion(el);
  el.style.overflow = "hidden";
  const { duration } = spring("smooth");
  const animation = el.animate(
    [
      { height: `${from}px`, opacity: 1 },
      { height: "0px", opacity: 0, paddingTop: "0px", paddingBottom: "0px" },
    ],
    { duration: Math.min(duration, 320), easing: "cubic-bezier(0.32, 0.72, 0, 1)", fill: "forwards" },
  );
  own(el, animation);
  animation.onfinish = () => el.remove();
  return animation;
}

/**
 * New content arriving in place of old, such as a link's details replacing "Reading the
 * link…": a short rise and fade, transform and opacity only. Returns the animation so a
 * newer transition can cancel it; null (no movement) with reduced motion or in a hidden window.
 */
export function settleIn(el: Element | null): Animation | null {
  if (!el || !canAnimate()) return null;
  const { easing, duration } = spring("smooth");
  return el.animate(
    [
      { opacity: 0, transform: "translateY(6px) scale(0.985)" },
      { opacity: 1, transform: "none" },
    ],
    { duration, easing },
  );
}

/** A small spring "pop", used when a count changes; none with reduced motion or in a hidden window. */
export function pop(el: HTMLElement) {
  if (!canAnimate()) return;
  const { easing, duration } = spring("bouncy");
  el.animate([{ transform: "scale(0.6)" }, { transform: "scale(1)" }], { duration, easing });
}

/** One short success acknowledgment, never a looping effect or a delayed status. */
export function completionFeedback(el: HTMLElement, completed: boolean) {
  cancelMotion(el);
  if (!completed || !canAnimate()) return;
  own(el, el.animate([{ opacity: 0.45, transform: "translateY(3px)" }, { opacity: 1, transform: "none" }], { duration: 220, easing: "ease-out" }));
}
