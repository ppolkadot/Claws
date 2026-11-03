// Static SVG markup authored here; never built from remote data.
const STROKE: Record<string, string> = {
  link: '<path d="M10 14a4.5 4.5 0 0 0 6.4 0l2.6-2.6a4.5 4.5 0 0 0-6.4-6.4l-1 1"/><path d="M14 10a4.5 4.5 0 0 0-6.4 0L5 12.6A4.5 4.5 0 0 0 11.4 19l1-1"/>',
  download: '<path d="M12 4.5v10.5"/><path d="m7.5 10.5 4.5 4.5 4.5-4.5"/><path d="M5 19.5h14"/>',
  list: '<path d="M9 6.5h11M9 12h11M9 17.5h11"/><path d="M4.5 6.5h.01M4.5 12h.01M4.5 17.5h.01"/>',
  grid: '<rect x="4.5" y="4.5" width="6" height="6" rx="1.6"/><rect x="13.5" y="4.5" width="6" height="6" rx="1.6"/><rect x="4.5" y="13.5" width="6" height="6" rx="1.6"/><rect x="13.5" y="13.5" width="6" height="6" rx="1.6"/>',
  sliders: '<path d="M4 8h8.5M17.5 8H20M4 16h2.5M11.5 16H20"/><circle cx="15" cy="8" r="2.5"/><circle cx="9" cy="16" r="2.5"/>',
  x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  minus: '<path d="M5.5 12h13"/>',
  sidebar: '<rect x="3.5" y="5" width="17" height="14" rx="3"/><path d="M9.5 5v14"/><path d="M5.8 8.5h1.6M5.8 11h1.6"/>',
  plus: '<path d="M12 5.5v13M5.5 12h13"/>',
  folder: '<path d="M3.5 7.5a2 2 0 0 1 2-2h3.4l2 2h7.6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
  trash: '<path d="M5 7h14"/><path d="M9.5 7V5.2c0-.4.3-.7.7-.7h3.6c.4 0 .7.3.7.7V7"/><path d="m7 7 .9 11.3c.1.7.6 1.2 1.3 1.2h5.6c.7 0 1.2-.5 1.3-1.2L17 7"/><path d="M10.5 11v5M13.5 11v5"/>',
  retry: '<path d="M19 12a7 7 0 1 1-2.1-5"/><path d="M19.2 4.8v4.4h-4.4"/>',
  check: '<path d="m5.5 12.5 4.2 4.2 8.8-9"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/>',
  music: '<path d="M9 17.5V6l10-2v11.5"/><circle cx="6.8" cy="17.7" r="2.2"/><circle cx="16.8" cy="15.7" r="2.2"/>',
  video: '<rect x="3.5" y="6" width="12.5" height="12" rx="2.6"/><path d="m16 10.5 4.5-2.5v8L16 13.5"/>',
  captions: '<rect x="3.5" y="5.5" width="17" height="13" rx="3"/><path d="M10 10.2a2 2 0 1 0 0 3.6M16 10.2a2 2 0 1 0 0 3.6"/>',
  open: '<path d="M14 5h5v5"/><path d="m19 5-8 8"/><path d="M17.5 14v3.5a1.5 1.5 0 0 1-1.5 1.5H6.5A1.5 1.5 0 0 1 5 17.5V8a1.5 1.5 0 0 1 1.5-1.5H10"/>',
  paste: '<rect x="5.5" y="5" width="13" height="15" rx="2.4"/><path d="M9.5 5V4.2c0-.4.3-.7.7-.7h3.6c.4 0 .7.3.7.7V5"/><path d="M9 11h6M9 14.5h4"/>',
  alert: '<path d="M12 4.2 21 19.5H3z"/><path d="M12 10v4.2M12 17h.01"/>',
  clock: '<circle cx="12" cy="12" r="8"/><path d="M12 7.5V12l3 2"/>',
  sparkles: '<path d="M12 4.5 13.6 9l4.4 1.6-4.4 1.6L12 16.7l-1.6-4.5L6 10.6 10.4 9z"/><path d="M18.5 15.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z"/>',
};

const FILLED: Record<string, string> = {
  play: '<path d="M8.2 5.6v12.8c0 .8.9 1.3 1.6.9l10-6.4c.6-.4.6-1.4 0-1.8l-10-6.4c-.7-.4-1.6.1-1.6.9z"/>',
  pause: '<rect x="6.5" y="5" width="3.7" height="14" rx="1.3"/><rect x="13.8" y="5" width="3.7" height="14" rx="1.3"/>',
};

export type IconName = keyof typeof STROKE | keyof typeof FILLED;

export function icon(name: IconName, size = 16): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.classList.add("icon");
  if (name in FILLED) {
    svg.setAttribute("fill", "currentColor");
    svg.innerHTML = FILLED[name];
  } else {
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.75");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.innerHTML = STROKE[name];
  }
  return svg;
}
