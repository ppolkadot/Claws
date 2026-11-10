/** CSS pixels the pointer travels before a press becomes a drag (Windows' drag box is 4 px). */
export const DRAG_THRESHOLD = 5;

/** How a native drag ended, as the backend reports it (file_drag.rs `DragOutcome`). */
export type DragOutcome = "dropped" | "cancelled" | "droppedOnSelf" | "notStarted";

export interface DragGesture {
  /** "pressed": primary button down on the handle, not moved far enough yet.
   *  "dragging": the native drag was requested and has not settled. */
  readonly phase: "idle" | "pressed" | "dragging";
  readonly x: number;
  readonly y: number;
  /** The mouse click that ends a press which became a drag is not a "show in folder". */
  readonly suppressClick: boolean;
}

export const idleGesture: DragGesture = { phase: "idle", x: 0, y: 0, suppressClick: false };

/** pointerdown. Only the primary mouse button starts anything, and never during a drag.
 *  Touch and pen are left to the click (show in folder): the native drag follows the
 *  mouse button, which a touch or pen contact on the page does not press. */
export function press(gesture: DragGesture, button: number, x: number, y: number, pointerType = "mouse"): DragGesture {
  if (gesture.phase === "dragging") return gesture;
  // A fresh state, so the tap's click shows the file even right after a mouse drag.
  if (pointerType !== "mouse") return idleGesture;
  if (button !== 0) return gesture;
  return { phase: "pressed", x, y, suppressClick: false };
}

/** pointermove. `start` is true exactly once: when the press becomes a native drag. */
export function move(gesture: DragGesture, x: number, y: number, buttons: number): { gesture: DragGesture; start: boolean } {
  if (gesture.phase !== "pressed") return { gesture, start: false };
  // The button came up where the page could not see it.
  if (!(buttons & 1)) return { gesture: { ...gesture, phase: "idle" }, start: false };
  if (Math.hypot(x - gesture.x, y - gesture.y) < DRAG_THRESHOLD) return { gesture, start: false };
  return { gesture: { ...gesture, phase: "dragging", suppressClick: true }, start: true };
}

/** pointerup, pointercancel or window blur. Ends a press; a running drag ends only by `settle`. */
export function release(gesture: DragGesture): DragGesture {
  return gesture.phase === "pressed" ? { ...gesture, phase: "idle" } : gesture;
}

/** The native drag call returned (any outcome or error). The OLE drag loop swallows the
 *  button release, so this, not pointerup, is what ends a drag. */
export function settle(gesture: DragGesture): DragGesture {
  return { ...gesture, phase: "idle" };
}

/** A click on the handle: Enter, Space (detail 0) or a plain click shows the file in its folder. */
export function revealsOnClick(gesture: DragGesture, detail: number): boolean {
  if (gesture.phase === "dragging") return false;
  return detail === 0 || !gesture.suppressClick;
}

/** Words for an outcome worth explaining, else null. A drop is never announced as a copy:
 *  Windows does not tell Claws whether the receiver copied or imported the file. */
export function outcomeMessage(outcome: DragOutcome): string | null {
  return outcome === "droppedOnSelf" ? "To copy the file, drop it on a folder or an app outside Claws." : null;
}
