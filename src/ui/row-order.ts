export interface Move<K> {
  key: K;
  /** Insert `key` before this one; null appends. */
  before: K | null;
}

/**
 * The moves that put a list's rows in `desired` order, applied in turn as insertBefore.
 * Rows in `leaving` (collapsing out) are neither moved nor counted, so a row on its way
 * out stays where it was instead of being pushed past every row below it. Rows already
 * in order produce no moves.
 */
export function orderMoves<K>(children: readonly K[], desired: readonly K[], leaving: ReadonlySet<K>): Move<K>[] {
  const list = [...children];
  const moves: Move<K>[] = [];
  let slot = 0;
  for (const key of desired) {
    while (slot < list.length && leaving.has(list[slot])) slot++;
    if (list[slot] !== key) {
      const before = slot < list.length ? list[slot] : null;
      const from = list.indexOf(key);
      if (from >= 0) list.splice(from, 1);
      list.splice(slot, 0, key);
      moves.push({ key, before });
    }
    slot++;
  }
  return moves;
}
