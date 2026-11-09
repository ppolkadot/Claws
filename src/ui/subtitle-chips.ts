import { canSelectSubtitle, offeredSubtitles } from "../transcript-options.js";
import { chipCheck } from "./components.js";

/**
 * The subtitle chips. At the limit (the backend's, transcript-options.ts) the unchosen
 * ones are marked unavailable and a click on one is stopped before the chip handles it;
 * chosen ones can always be cleared. `selected` is changed in place, as chipCheck does.
 */
export function subtitleChips(
  tracks: readonly { code: string; name: string }[],
  selected: Set<string>,
  onChange: (selected: Set<string>) => void,
): HTMLElement {
  const offered = offeredSubtitles(tracks);
  const chips = chipCheck("Subtitles", offered.map((s) => ({ value: s.code, label: s.name })), selected, onChange);
  chips.querySelectorAll<HTMLElement>(".chip").forEach((chip, i) => {
    if (!canSelectSubtitle(selected, offered[i].code)) chip.setAttribute("aria-disabled", "true");
  });
  chips.addEventListener(
    "click",
    (event) => {
      if ((event.target as Element).closest(".chip")?.getAttribute("aria-disabled") === "true") event.stopPropagation();
    },
    true,
  );
  return chips;
}
