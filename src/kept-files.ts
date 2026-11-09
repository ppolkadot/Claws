// Subtitle files a download kept in its work folder because their names were taken in the
// output folder: what the Library and the Queue say about them. Pure logic, no DOM, so Node
// tests can run it (scripts/tests/kept-files.test.mjs). The backend (history.rs) decides what
// is kept and where; this only words it.

/** The saved record on a Library entry; `sources` are paths inside the work folder. */
export interface KeptRecord {
  workspace: { path: string };
  sources: string[];
}

export type KeptStatus = "shown" | "offline" | "missing" | "unverified";

/** The answer of "Show kept files". */
export interface KeptRevealResult {
  status: KeptStatus;
  /** The folder the kept files are in. */
  folder: string;
  files: number;
}

/** What the last look for kept files of older downloads found. */
export interface KeptRepairReport {
  attached: number;
  /** Known output folders that could not be read, so their downloads were not checked. */
  inaccessible: string[];
  ignored: number;
}

interface WithKept {
  keptFiles?: KeptRecord | null;
}

const files = (count: number) => (count === 1 ? "1 subtitle file" : `${count} subtitle files`);

export function keptCount(item: WithKept | undefined): number {
  return item?.keptFiles?.sources.length ?? 0;
}

/** The persistent line that marks an entry with kept files; null when it has none. */
export function keptLabel(item: WithKept | undefined): string | null {
  const count = keptCount(item);
  return count > 0 ? `${files(count)} kept separately` : null;
}

/** Changes whenever what a row or tile shows about kept files changes. */
export function keptKey(item: WithKept | undefined): string {
  return keptCount(item) > 0 ? `${keptCount(item)}@${item?.keptFiles?.workspace.path}` : "";
}

/**
 * What to tell the user after Show kept files. A folder that is offline, gone or not provably
 * this download's is an error with its own explanation, never mistaken for a shown file.
 */
export function keptOutcome(result: KeptRevealResult): { tone: "info" | "error"; text: string } {
  switch (result.status) {
    case "shown":
      return { tone: "info", text: `Showing ${files(result.files)} kept in ${result.folder}.` };
    case "offline":
      return { tone: "error", text: `The folder with the kept subtitle files cannot be reached right now (${result.folder}). Reconnect the drive or folder and try again.` };
    case "missing":
      return { tone: "error", text: `The kept subtitle files are no longer in ${result.folder}. They were moved or deleted.` };
    case "unverified":
      return { tone: "error", text: `Claws cannot confirm that ${result.folder} is the folder it kept these files in, so it did not open it.` };
  }
}

/** Added to the remove confirmation: what stays whichever option is chosen. */
export function keptRemovalWarning(item: WithKept | undefined): string | null {
  const count = keptCount(item);
  if (count === 0 || !item?.keptFiles) return null;
  const where = item.keptFiles.workspace.path;
  const stay = count === 1 ? "A subtitle file" : `${count} subtitle files`;
  return `${stay} Claws kept separately (the name was already taken) will stay in ${where}, whichever option you choose. Once this entry is gone, Claws can no longer show it.`;
}

/** Added to the Clear Library confirmation: every entry with kept files loses its way back. */
export function keptClearWarning(items: readonly (WithKept | undefined)[]): string | null {
  const count = items.filter((item) => keptCount(item) > 0).length;
  if (count === 0) return null;
  const who = count === 1 ? "1 download has kept subtitle files" : `${count} downloads have kept subtitle files`;
  return `${who} (their names were already taken). Those files stay in their work folders, and once the entries are gone Claws can no longer show them.`;
}

/** The Library's note about folders that could not be checked; null when there is none. */
export function repairNote(report: KeptRepairReport | null | undefined): string | null {
  const folders = report?.inaccessible ?? [];
  if (folders.length === 0) return null;
  const names = folders.length > 3 ? `${folders.slice(0, 3).join(", ")} and ${folders.length - 3} more` : folders.join(", ");
  return `Claws could not check ${folders.length === 1 ? "this folder" : "these folders"} for subtitle files kept from earlier downloads: ${names}.`;
}
