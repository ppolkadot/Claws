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
