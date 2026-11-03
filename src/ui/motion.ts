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

/** Grows an element from zero height into place, as a new row arriving in a list. */
export function expandIn(el: HTMLElement) {
  if (reducedMotion()) return;
  const height = el.getBoundingClientRect().height;
  const { easing, duration } = spring("smooth");
  el.animate(
    [
      { height: "0px", opacity: 0, transform: "scale(0.98)" },
      { height: `${height}px`, opacity: 1, transform: "none" },
    ],
    { duration, easing },
  );
}

/** Collapses an element to zero height, then removes it. */
export function collapseOut(el: HTMLElement) {
  if (reducedMotion()) return el.remove();
  const height = el.getBoundingClientRect().height;
  el.style.overflow = "hidden";
  const { duration } = spring("smooth");
  const animation = el.animate(
    [
      { height: `${height}px`, opacity: 1 },
      { height: "0px", opacity: 0, paddingTop: "0px", paddingBottom: "0px" },
    ],
    { duration: Math.min(duration, 320), easing: "cubic-bezier(0.32, 0.72, 0, 1)", fill: "forwards" },
  );
  animation.onfinish = () => el.remove();
}

/** A small spring "pop", used when a count changes. */
export function pop(el: HTMLElement) {
  if (reducedMotion()) return;
  const { easing, duration } = spring("bouncy");
  el.animate([{ transform: "scale(0.6)" }, { transform: "scale(1)" }], { duration, easing });
}
