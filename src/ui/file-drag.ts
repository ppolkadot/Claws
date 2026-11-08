import { api, errorMessage } from "../api";
import { button, toast } from "./components";

/** Native drag on pointer movement; keyboard activation retains Show in folder. */
export function fileDragButton(id: string): HTMLButtonElement {
  const control = button({ label: "Drag file", title: "Drag the saved file into an editor or folder. Press Enter to show it in its folder.",
    variant: "plain", onClick: (event) => {
      if (!busy && (event.detail === 0 || !suppressPointerClick)) void api.revealDownload(id).catch(fail);
    } });
  const fail = (e: unknown) => toast(errorMessage(e), { tone: "error" });
  let busy = false;
  let suppressPointerClick = false;
  let gesture: AbortController | null = null;
  control.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || busy) return;
    suppressPointerClick = false;
    gesture?.abort();
    gesture = new AbortController();
    const { signal } = gesture;
    const x = event.clientX, y = event.clientY;
    const stop = () => gesture?.abort();
    document.addEventListener("pointerup", stop, { once: true, signal });
    document.addEventListener("pointercancel", stop, { once: true, signal });
    window.addEventListener("blur", stop, { once: true, signal });
    document.addEventListener("pointermove", (move) => {
      if (!control.isConnected || !(move.buttons & 1)) return stop();
      if (Math.hypot(move.clientX - x, move.clientY - y) < 5) return;
      stop();
      busy = true;
      suppressPointerClick = true;
      control.classList.add("file-drag-active");
      void api.dragDownload(id).catch(fail).finally(() => {
        control.classList.remove("file-drag-active");
        busy = false;
      });
    }, { signal });
  });
  return control;
}
