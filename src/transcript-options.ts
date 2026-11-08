// Transcript choices: pure logic, no DOM, so Node tests can run it
// (scripts/tests/transcript-options.test.mjs).
//
// A transcript is made from one of the site's own caption tracks (transcript.rs); Claws does
// not recognise speech. Human and automatic captions of the same language are different
// tracks, so a choice is keyed by both the code and the kind.

/** What a transcript is saved as (transcript.rs `TranscriptFormat`). */
export type TranscriptFormat = "txt" | "txtTimestamps" | "srt" | "vtt";

/** One caption track (probe.rs `SubtitleLang`). */
export interface CaptionTrack {
  code: string;
  name: string;
  auto: boolean;
}

/** The parts of a lookup this module reads (probe.rs `ProbeResult`). */
export interface CaptionSource {
  kind: "video" | "playlist";
  subtitles: CaptionTrack[];
  /** Absent (or null) in links saved before transcripts existed. */
  captions?: CaptionTrack[] | null;
}

export const TRANSCRIPT_FORMATS: { value: TranscriptFormat; label: string; hint: string }[] = [
  { value: "txt", label: "Text", hint: "Plain text, one caption per line, with a blank line at long pauses." },
  { value: "txtTimestamps", label: "Text with timestamps", hint: "Plain text with the time each caption starts, like [00:01:02]." },
  { value: "srt", label: "SRT", hint: "A clean SubRip subtitle file with the original timings." },
  { value: "vtt", label: "VTT", hint: "A clean WebVTT subtitle file with the original timings." },
];

export interface CaptionChoices {
  human: CaptionTrack[];
  automatic: CaptionTrack[];
  /** False when there is nothing to make a transcript from (or for a playlist). */
  available: boolean;
}

/**
 * The tracks a transcript can be made from. Links saved before caption lists existed fall
 * back to the human tracks of their subtitle list: its automatic entries were picked by
 * language and can be machine translations, which are not a transcript of what was said.
 */
export function captionChoices(result: CaptionSource): CaptionChoices {
  if (result.kind !== "video") return { human: [], automatic: [], available: false };
  const tracks = result.captions ?? result.subtitles.filter((s) => !s.auto);
  const human = tracks.filter((t) => !t.auto);
  const automatic = tracks.filter((t) => t.auto);
  return { human, automatic, available: human.length + automatic.length > 0 };
}

export function captionKey(track: Pick<CaptionTrack, "code" | "auto">): string {
  return `${track.auto ? "auto" : "human"}:${track.code}`;
}

export function parseCaptionKey(key: string): { code: string; auto: boolean } | null {
  const match = /^(auto|human):(.+)$/.exec(key);
  return match ? { code: match[2], auto: match[1] === "auto" } : null;
}

/** A language code's base language, lower case: `en-US` and `en-orig` are both `en`. */
function baseLanguage(code: string): string {
  return code.toLowerCase().split(/[-_]/)[0];
}

/** The human track for `language`: the exact code first, then one of the same base language. */
function humanFor(human: CaptionTrack[], language: string): CaptionTrack | undefined {
  const exact = human.find((t) => t.code.toLowerCase() === language.toLowerCase());
  return exact ?? human.find((t) => baseLanguage(t.code) === baseLanguage(language));
}

/**
 * The track chosen until the user picks one. Sites list human tracks alphabetically (the
 * probe keeps the order it gets), so the first is often not the video's language. In order:
 * human captions in the video's own language (named by YouTube's `-orig` automatic track);
 * human captions in one of `preferred` (the user's languages, e.g. navigator.languages),
 * then English; any human captions; the original-language automatic track; any track.
 */
export function defaultCaption(choices: CaptionChoices, preferred: readonly string[] = []): string | null {
  const original = choices.automatic.find((t) => t.code.endsWith("-orig"));
  const spoken = original ? original.code.replace(/-orig$/, "") : null;
  let pick: CaptionTrack | undefined = spoken ? humanFor(choices.human, spoken) : undefined;
  for (const language of [...preferred, "en"]) pick ??= humanFor(choices.human, language);
  pick ??= choices.human[0] ?? original ?? choices.automatic[0];
  return pick ? captionKey(pick) : null;
}

/** The chosen track while a refresh still offers it, otherwise the default. */
export function keptCaption(key: string | null, choices: CaptionChoices, preferred: readonly string[] = []): string | null {
  const all = [...choices.human, ...choices.automatic];
  return key && all.some((t) => captionKey(t) === key) ? key : defaultCaption(choices, preferred);
}

/** The options a transcript is queued with (downloader.rs `DownloadOptions`). */
export interface TranscriptDownloadOptions {
  mode: "transcript";
  maxHeight: null;
  videoPreset: "compatible";
  audioFormat: string;
  subtitleLangs: string[];
  autoSubs: boolean;
  embedSubs: boolean;
  fastSave: boolean;
  transcriptFormat: TranscriptFormat;
}

/**
 * One caption track and the format; every media choice at its default, as the backend
 * resets them (`DownloadOptions::validate`), so the same transcript is one queue entry.
 */
export function transcriptOptions(key: string | null, format: TranscriptFormat): TranscriptDownloadOptions | null {
  const track = key ? parseCaptionKey(key) : null;
  if (!track) return null;
  return {
    mode: "transcript",
    maxHeight: null,
    videoPreset: "compatible",
    audioFormat: "mp3",
    subtitleLangs: [track.code],
    autoSubs: track.auto,
    embedSubs: false,
    fastSave: false,
    transcriptFormat: format,
  };
}

/** The format names the Library uses (transcript.rs `TranscriptFormat::label`). */
const FORMAT_LABELS: Record<TranscriptFormat, string> = { txt: "TXT", txtTimestamps: "TXT with timestamps", srt: "SRT", vtt: "VTT" };

/** English language names, the same list as transcript.rs `language_name`. */
const LANGUAGE_NAMES: Record<string, string> = {
  en: "English", "en-US": "English (US)", "en-GB": "English (UK)", es: "Spanish", "es-419": "Spanish (Latin America)",
  pt: "Portuguese", "pt-BR": "Portuguese (Brazil)", "pt-PT": "Portuguese (Portugal)", fr: "French", de: "German",
  it: "Italian", nl: "Dutch", ja: "Japanese", ko: "Korean", zh: "Chinese", "zh-Hans": "Chinese (Simplified)",
  "zh-Hant": "Chinese (Traditional)", "zh-CN": "Chinese (China)", "zh-TW": "Chinese (Taiwan)", ru: "Russian",
  uk: "Ukrainian", pl: "Polish", cs: "Czech", sv: "Swedish", da: "Danish", no: "Norwegian", nb: "Norwegian",
  fi: "Finnish", el: "Greek", tr: "Turkish", ar: "Arabic", he: "Hebrew", iw: "Hebrew", fa: "Persian", hi: "Hindi",
  bn: "Bengali", ur: "Urdu", th: "Thai", vi: "Vietnamese", id: "Indonesian", ms: "Malay", ro: "Romanian", hu: "Hungarian",
};

/** A caption language's English name (transcript.rs `language_name`); the code when unknown. */
export function languageName(code: string): string {
  const plain = code.replace(/-orig$/, "");
  return LANGUAGE_NAMES[plain] ?? LANGUAGE_NAMES[plain.split(/[-_]/)[0]] ?? plain;
}

type TranscriptChoice = { subtitleLangs: string[]; autoSubs: boolean; transcriptFormat?: TranscriptFormat };

/**
 * A transcript job described as the Library describes it (transcript.rs `quality`), e.g.
 * "Transcript · English (auto-generated) · TXT", so the queue and the Library agree.
 */
export function transcriptText(options: TranscriptChoice): string {
  const code = options.subtitleLangs[0] ?? "";
  const original = code.endsWith("-orig");
  const kind = original && options.autoSubs ? " (original, auto-generated)" : original ? " (original)" : options.autoSubs ? " (auto-generated)" : "";
  return `Transcript · ${languageName(code)}${kind} · ${FORMAT_LABELS[options.transcriptFormat ?? "txt"]}`;
}

/** The end of a transcript's file name (transcript.rs `artifact_name`): " [en auto, timestamps].txt". */
export function transcriptFileSuffix(options: TranscriptChoice): string {
  const format = options.transcriptFormat ?? "txt";
  const label = `${options.subtitleLangs[0] ?? ""}${options.autoSubs ? " auto" : ""}${format === "txtTimestamps" ? ", timestamps" : ""}`;
  const ext = format === "srt" ? "srt" : format === "vtt" ? "vtt" : "txt";
  return ` [${label}].${ext}`;
}
