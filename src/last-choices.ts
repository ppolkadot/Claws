// The choices of the last download, offered again for the next link. Kept in localStorage
// like the sidebar flag: losing them is harmless, so every read and write is guarded.

export interface LastChoices {
  mode: "video" | "audio" | "transcript";
  audioFormat: string;
  /** The quality cap by the shorter side; null is Best. */
  height: number | null;
  videoPreset: "compatible" | "original";
  fastSave: boolean;
}

export const LAST_CHOICES_KEY = "claws.lastChoices";

const MODES = ["video", "audio", "transcript"];
const PRESETS = ["compatible", "original"];

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;

export function parseLastChoices(text: string | null): LastChoices | null {
  if (!text) return null;
  try {
    const value = JSON.parse(text) as Partial<LastChoices> | null;
    if (!value || typeof value !== "object") return null;
    const height = value.height ?? null;
    if (!MODES.includes(value.mode as string) || !PRESETS.includes(value.videoPreset as string)) return null;
    if (typeof value.audioFormat !== "string" || typeof value.fastSave !== "boolean") return null;
    if (height !== null && !(Number.isInteger(height) && height > 0)) return null;
    return { mode: value.mode!, audioFormat: value.audioFormat, height, videoPreset: value.videoPreset!, fastSave: value.fastSave };
  } catch {
    return null;
  }
}

function defaultStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function loadLastChoices(storage: Storage | null = defaultStorage()): LastChoices | null {
  try {
    return storage ? parseLastChoices(storage.getItem(LAST_CHOICES_KEY)) : null;
  } catch {
    return null;
  }
}

export function saveLastChoices(choices: LastChoices, storage: Storage | null = defaultStorage()) {
  try {
    storage?.setItem(LAST_CHOICES_KEY, JSON.stringify(choices));
  } catch {
  }
}

export interface LinkOffer {
  heights: number[];
  audioFormats: string[];
  /** Whether this link can be saved as a transcript (it has captions and is not a playlist). */
  transcript: boolean;
}

/**
 * The remembered choices as far as this link allows them. A cap the source does not offer
 * goes back to Best, a transcript the link cannot give goes back to video, and an audio
 * format that no longer exists goes back to the first one offered.
 */
export function applyLastChoices(last: LastChoices, offer: LinkOffer): LastChoices {
  return {
    mode: last.mode === "transcript" && !offer.transcript ? "video" : last.mode,
    audioFormat: offer.audioFormats.includes(last.audioFormat) ? last.audioFormat : (offer.audioFormats[0] ?? last.audioFormat),
    height: last.height !== null && offer.heights.includes(last.height) ? last.height : null,
    videoPreset: last.videoPreset,
    fastSave: last.fastSave,
  };
}
