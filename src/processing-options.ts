// Mirrors ProcessingPolicy in download_policy.rs.
// Contract: Fast Save skips the optional passes that can rewrite the finished file, that is
// writing metadata, embedding subtitles and embedding cover art. Necessary work stays:
// audio conversion, merging video with audio and any container repair the download needs.

export interface FastSaveChoices {
  mode: "video" | "audio" | "transcript";
  audioFormat: string;
  metadata: boolean;
  /** Video only: subtitles are chosen and set to be embedded rather than saved beside it. */
  embeddedSubs: boolean;
  fastSave: boolean;
}

/** Whether cover art is embedded when Fast Save is off: MP3 and M4A audio downloads. */
export function embedsCoverArt(mode: string, audioFormat: string): boolean {
  return mode === "audio" && (audioFormat === "mp3" || audioFormat === "m4a");
}

/** The optional passes Fast Save skips for these choices, in the order they are named. */
export function skippedByFastSave(c: FastSaveChoices): string[] {
  return [
    c.metadata ? "writing title, uploader and date into the file" : null,
    c.embeddedSubs ? "embedding subtitles (they are saved as .srt files instead)" : null,
    embedsCoverArt(c.mode, c.audioFormat) ? "embedding cover art (MP3 and M4A files are saved without it)" : null,
  ].filter((item): item is string => item !== null);
}

function joined(items: string[]): string {
  return items.length < 3 ? items.join(" and ") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

export function fastSaveHint(c: FastSaveChoices): string {
  const skipped = skippedByFastSave(c);
  if (skipped.length === 0) return "Nothing optional to skip with these choices. Conversion and merging, where the format needs them, still happen.";
  const list = joined(skipped);
  return c.fastSave
    ? `Skips ${list}. The same video and audio are downloaded, and any conversion, merge or container repair the format needs still happens.`
    : `Turn on to skip ${list}. Each can rewrite the whole file after it downloads; conversion, merging and any container repair the format needs still happen.`;
}
