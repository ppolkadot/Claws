// The Library reuses a tile while nothing it shows or acts on has changed, rebuilds it once when
// something did, and keeps the tiles that stay in place in the grid. Imports only the pure row
// ordering, so the Node tests can run it (scripts/tests/library-cache.test.mjs).

import { orderMoves } from "./ui/row-order.js";

/**
 * The entry values a tile reads, directly or through the handlers it captures (views/library.ts):
 * the text, artwork and fallback icon; the drag, open, show and remove actions; whether the file
 * is provably the one Claws saved (Delete File Too); and the kept subtitle files.
 */
export interface TileSource {
  id: string;
  title: string;
  uploader: string | null;
  duration: number | null;
  thumbnail: string | null;
  thumbnailLocal: string | null;
  filePath: string | null;
  fileSize: number | null;
  mode: string;
  quality: string;
  finishedAt: number;
  /** Only whether it is there matters to the tile, not what it is. */
  fileIdentity?: unknown;
  keptFiles?: { workspace: { path: string }; sources: unknown[] } | null;
}

/**
 * Every value the tile consumes, as JSON, so two different entries can never share a signature
 * by value alone (a separator inside a title, `null` against the text "null"). The entry's link
 * and anything else the tile does not read is left out, so it cannot cause a rebuild.
 */
export function tileSignature(item: TileSource): string {
  return JSON.stringify([
    item.title,
    item.uploader,
    item.duration,
    item.thumbnailLocal,
    item.thumbnail,
    item.filePath,
    item.fileSize,
    item.mode,
    item.quality,
    item.finishedAt,
    Boolean(item.fileIdentity),
    item.keptFiles ? [item.keptFiles.workspace.path, item.keptFiles.sources.length] : null,
  ]);
}

/** Tiles by entry id, each kept while its signature is unchanged. */
export class TileCache<El> {
  private readonly built = new Map<string, { key: string; el: El }>();

  get size(): number {
    return this.built.size;
  }

  get(item: TileSource, build: () => El): El {
    const key = tileSignature(item);
    const cached = this.built.get(item.id);
    if (cached && cached.key === key) return cached.el;
    const el = build();
    this.built.set(item.id, { key, el });
    return el;
  }

  /** Forgets every tile whose entry is not in `live`. */
  prune(live: ReadonlySet<string>) {
    for (const id of this.built.keys()) if (!live.has(id)) this.built.delete(id);
  }
}

/**
 * Puts `wanted` in `parent`, in order, taking out only what is not wanted and moving only what is
 * out of place. A tile that stays is never removed and added again: that would drop the keyboard
 * focus from the one a user is on, which `replaceChildren` does.
 */
export function reconcileChildren<E extends Element>(
  parent: { readonly children: ArrayLike<E>; insertBefore(node: E, before: E | null): unknown },
  wanted: readonly E[],
) {
  const keep = new Set<E>(wanted);
  const present = Array.from(parent.children);
  for (const el of present) if (!keep.has(el)) el.remove();
  for (const move of orderMoves(present.filter((el) => keep.has(el)), wanted, new Set<E>())) parent.insertBefore(move.key, move.before);
}
