import { api, errorMessage } from "../api";
import { button, toast } from "./components";
import { idleGesture, move, outcomeMessage, press, release, revealsOnClick, settle } from "./drag-gesture";

const fail = (e: unknown) => toast(errorMessage(e), { tone: "error" });

/**
 * A handle that starts Windows' native, copy-only drag of a saved file once the pointer
 * moves (the backend resolves the file from the id). Enter, Space or a plain click shows
 * the file in its folder instead, alongside the row's own Open and Show in folder buttons.
 * Success is never claimed: Windows does not tell Claws what the receiver did.
 */
export function fileDragButton(id: string): HTMLButtonElement {
  let gesture = idleGesture;
  let listening: AbortController | null = null;
  const control = button({
    label: "Drag file",
    title: "Drag to copy the file into a folder or an editor. Enter, Space or a click shows it in its folder.",
    variant: "plain",
    onClick: (event) => {
      if (revealsOnClick(gesture, event.detail)) void api.revealDownload(id).catch(fail);
    },
  });
  control.classList.add("file-drag");
  const stopListening = () => {
    listening?.abort();
    listening = null;
  };
  const end = () => {
    gesture = release(gesture);
    stopListening();
  };
  // Only the native drag exports the file; never an HTML drag of the page's content.
  control.addEventListener("dragstart", (event) => event.preventDefault());
  control.addEventListener("pointerdown", (event) => {
    gesture = press(gesture, event.button, event.clientX, event.clientY, event.pointerType);
    if (gesture.phase !== "pressed") return;
    stopListening();
    listening = new AbortController();
    const { signal } = listening;
    document.addEventListener("pointerup", end, { signal });
    document.addEventListener("pointercancel", end, { signal });
    window.addEventListener("blur", end, { signal });
    document.addEventListener(
      "pointermove",
      (moved) => {
        if (!control.isConnected) return end(); // the view went away mid-press
        const step = move(gesture, moved.clientX, moved.clientY, moved.buttons);
        gesture = step.gesture;
        if (gesture.phase === "idle") return stopListening();
        if (!step.start) return;
        stopListening();
        // Pickup feedback lasts exactly as long as the native operation.
        control.classList.add("file-drag-active");
        void api
          .dragDownload(id)
          .then((outcome) => {
            const message = outcomeMessage(outcome);
            if (message) toast(message);
          })
          .catch(fail)
          .finally(() => {
            gesture = settle(gesture);
            control.classList.remove("file-drag-active");
          });
      },
      { signal },
    );
  });
  return control;
}
