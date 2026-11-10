// yt-dlp's `-S res:N` caps the shorter side of the picture, so a choice's value is a
// shorter side: a portrait 1080×1920 stream is the "1080p" choice. Choices come from the
// sizes the source offers (probe.rs `resolutions`), never a fixed ladder, and a class name
// (4K, 5K, 6K, 8K) appears only when the dimensions reach it. Nothing is ever upscaled.

/** One picture size a source offers (probe.rs `Resolution`). */
export interface Resolution {
  /** The shorter side, which a choice caps. */
  shortSide: number;
  /** Null when the site gave only a height. */
  width: number | null;
  height: number | null;
  vcodec?: string | null;
  dynamicRange?: string | null;
  fps?: number | null;
}

/** What a video download keeps (download_policy.rs `VideoPreset`). */
export type VideoPreset = "compatible" | "original" | "editingProRes";

export interface QualityChoice {
  /** The shorter side to cap at; Best (no cap) is not listed here. */
  value: number;
  label: string;
  hint: string;
}

/** The smallest size offered (probe.rs MIN_SHORT_SIDE). */
const MIN_SHORT_SIDE = 144;

/**
 * Caps offered where there are no per-item sizes (a batch, a flat playlist). Each item is
 * saved at the largest size it has up to the cap, so a cap above an item's size is harmless.
 */
const GENERIC_CAPS = [4320, 2880, 2160, 1440, 1080, 720, 480, 360];

/**
 * The size class a picture's dimensions justify, or null. Orientation does not matter;
 * a picture without a known width gets none, since its height alone proves nothing.
 *
 * Every class needs BOTH its long and its short side, so a panorama is never named by its
 * width alone: 8K 7680×4320, 6K 5760×3160 (DCI 6144×3160 included), 5K 5120×2880, 4K
 * 3840×2160 (DCI 4096×2160 included). An ultrawide 7680×3200 is 6K, not 8K; 5760×1080 and
 * 3840×1600 get no class. Nothing is named above what it reaches.
 */
export function resolutionClass(width: number | null | undefined, height: number | null | undefined): string | null {
  if (!width || !height) return null;
  const long = Math.max(width, height);
  const short = Math.min(width, height);
  if (long >= 7680 && short >= 4320) return "8K";
  if (long >= 5760 && short >= 3160) return "6K";
  if (long >= 5120 && short >= 2880) return "5K";
  if (long >= 3840 && short >= 2160) return "4K";
  return null;
}

/** "4320p · 7680×4320 · 8K", "1080p · 1080×1920 portrait", or "2160p" without a width. */
export function describeResolution(r: Resolution): string {
  const parts = [`${r.shortSide}p`];
  if (r.width && r.height) {
    parts.push(`${r.width}×${r.height}${r.height > r.width ? " portrait" : ""}`);
    const size = resolutionClass(r.width, r.height);
    if (size) parts.push(size);
  }
  return parts.join(" · ");
}

const capHint = (value: number) => `Up to ${value}p`;

/** The generic caps, for choices that apply to items whose sizes are not known. */
export function capChoices(): QualityChoice[] {
  return GENERIC_CAPS.map((value) => ({ value, label: capHint(value), hint: `The largest size each video has, up to ${value}p on its shorter side` }));
}

/** True when a lookup reports its exact sizes (probe.rs `resolutions`), not only heights. */
export function hasExactSizes(probe: { resolutions?: Resolution[] | null }): boolean {
  return (probe.resolutions ?? []).some((r) => r.shortSide >= MIN_SHORT_SIDE);
}

/**
 * Size choices for a lookup: one per size the source offers. A list with no sizes (a flat
 * playlist) gets the generic caps.
 *
 * A link saved before sizes were recorded in detail has only `heights`, which were each
 * format's raw height: for portrait video that is the longer side, not the shorter side
 * `res:N` caps. So those values are offered as caps ("Up to 1920p"), which stays true for
 * either orientation, and never as exact sizes. The refresh that follows replaces them.
 */
export function qualityChoices(probe: { heights: number[]; resolutions?: Resolution[] | null }, playlist: boolean): QualityChoice[] {
  if (hasExactSizes(probe)) {
    return (probe.resolutions ?? [])
      .filter((r) => r.shortSide >= MIN_SHORT_SIDE)
      .map((r) => ({ value: r.shortSide, label: describeResolution(r), hint: capHint(r.shortSide) }));
  }
  const heights = probe.heights.filter((h) => h >= MIN_SHORT_SIDE);
  if (heights.length > 0) {
    return heights.map((value) => ({ value, label: capHint(value), hint: `The largest size this video has, up to ${value}p on its shorter side` }));
  }
  return playlist ? capChoices() : [];
}

/**
 * The size cap to keep when the choices change (a refreshed lookup): the same value when it
 * is still offered, otherwise Best. A cap that no chip shows would still be sent, so it is
 * never kept.
 */
export function keptQuality(value: number | null, choices: QualityChoice[]): number | null {
  return value !== null && choices.some((c) => c.value === value) ? value : null;
}

/**
 * The presets offered. The editing preset is reserved until processing exists: the backend
 * refuses it, so it is not shown.
 */
export const VIDEO_PRESETS: { value: Exclude<VideoPreset, "editingProRes">; label: string; hint: string }[] = [
  {
    value: "compatible",
    label: "Compatible",
    hint: "Prefers H.264 video and AAC audio in MP4 when the site offers them; otherwise another codec may be used.",
  },
  {
    value: "original",
    label: "Original",
    hint: "Keeps the site's best streams without re-encoding; the file may be MKV and some editors cannot open every codec.",
  },
];

const PRESET_NAMES: Record<VideoPreset, string | null> = { compatible: null, original: "Original", editingProRes: "Editing" };

/** A queued video's choices: its size cap, then its preset unless it is the default. */
export function videoOptionsText(maxHeight: number | null, preset: VideoPreset | undefined): string {
  const size = maxHeight ? `Video up to ${maxHeight}p` : "Video, best quality";
  const name = preset ? PRESET_NAMES[preset] : null;
  return name ? `${size} · ${name}` : size;
}
