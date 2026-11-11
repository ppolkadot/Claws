export interface SearchableItem {
  title: string;
  uploader: string | null;
  matchOrigin?: { spotifyTitle: string | null; spotifyArtists: string[] } | null;
}

/**
 * Whether the library search box finds this entry: the saved title and uploader, and for a
 * Spotify match also the Spotify title and artists. Trimmed, case-insensitive substring.
 */
export function matchesLibrarySearch(item: SearchableItem, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  const origin = item.matchOrigin;
  return [item.title, item.uploader, origin?.spotifyTitle, ...(origin?.spotifyArtists ?? [])].some((text) => (text ?? "").toLowerCase().includes(needle));
}

export type LibraryKind = "all" | "video" | "audio" | "transcript";
export type LibraryOrder = "date" | "title" | "size";

export const LIBRARY_KINDS: { value: LibraryKind; label: string }[] = [
  { value: "all", label: "All" },
  { value: "video", label: "Video" },
  { value: "audio", label: "Audio" },
  { value: "transcript", label: "Transcript" },
];

export const LIBRARY_ORDERS: { value: LibraryOrder; label: string }[] = [
  { value: "date", label: "Newest first" },
  { value: "title", label: "Title" },
  { value: "size", label: "Largest first" },
];

export function matchesLibraryKind(item: { mode: string }, kind: LibraryKind): boolean {
  return kind === "all" || item.mode === kind;
}

export interface SortableItem {
  id: string;
  title: string;
  fileSize: number | null;
  finishedAt: number;
}

/**
 * The entries in the chosen order, as a new array. Newest first by date; titles from A to Z
 * without regard to case; largest first by size, with entries of unknown size last. Ties
 * fall back to newest first, then to the id, so the order never depends on the input order.
 */
export function sortLibrary<T extends SortableItem>(items: readonly T[], order: LibraryOrder): T[] {
  const newest = (a: T, b: T) => b.finishedAt - a.finishedAt || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const compare: (a: T, b: T) => number =
    order === "title"
      ? (a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base", numeric: true }) || newest(a, b)
      : order === "size"
        ? (a, b) => {
            if (a.fileSize == null || b.fileSize == null) return a.fileSize == null && b.fileSize == null ? newest(a, b) : a.fileSize == null ? 1 : -1;
            return b.fileSize - a.fileSize || newest(a, b);
          }
        : newest;
  return [...items].sort(compare);
}
