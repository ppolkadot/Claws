import type { SpotifyMatchState } from "../spotify-match.js";
import { formatDuration } from "../format.js";
import { button } from "../ui/components.js";
import { append, h } from "../ui/dom.js";

export interface SpotifyMatchActions {
  setQuery(query: string): void;
  search(): void;
  select(videoId: string): void;
  confirm(): void;
  retry(): void;
}

/** Metadata is always text; candidate selection never starts a download. */
export function renderSpotifyMatch(state: SpotifyMatchState, actions: SpotifyMatchActions): HTMLElement {
  const busy = state.stage === "resolving" || state.stage === "searching" || state.stage === "probing";
  const track = state.track;
  const summary = h("div", { class: "spotify-identity group" },
    h("span", { class: "row-hint", text: "Spotify track" }),
    h("strong", { text: track?.title ?? "Identify this song" }),
    h("span", { class: "row-hint", text: [track?.artists.join(", "), formatDuration(track?.duration ?? null)].filter(Boolean).join(" · ") }),
  );
  if (track?.artwork) {
    const image = h("img", { class: "spotify-art", attrs: { src: track.artwork, alt: "", referrerpolicy: "no-referrer", draggable: false } });
    image.addEventListener("error", () => image.remove());
    summary.prepend(image);
  }
  const query = h("input", { class: "text-input spotify-query", attrs: { id: "spotify-query", type: "text", "aria-label": "Search YouTube", maxlength: 300, placeholder: "Artist and song title" } });
  query.value = state.query;
  query.disabled = state.stage === "resolving";
  query.addEventListener("input", () => actions.setQuery(query.value));
  query.addEventListener("keydown", (event) => { if (event.key === "Enter" && !busy) { event.preventDefault(); actions.search(); } });
  const search = button({ label: "Find on YouTube", onClick: actions.search });
  search.disabled = busy || !track || !state.query.trim();
  const panel = h("div", { class: "spotify-match" }, summary,
    h("p", { class: "row-hint", text: "Find a recording on YouTube, then choose a match. Audio is downloaded from YouTube. The recording may differ from the Spotify version." }),
    h("div", { class: "spotify-search" }, query, search),
  );
  if (state.error) panel.appendChild(h("div", { class: "notice notice-error", text: state.error, attrs: { role: "alert" } }));
  for (const warning of state.warnings) panel.appendChild(h("p", { class: "row-hint", text: warning }));
  if (busy) panel.appendChild(h("p", { attrs: { role: "status" }, text: state.stage === "resolving" ? "Reading Spotify track…" : state.stage === "probing" ? "Checking the selected recording…" : "Finding YouTube recordings…" }));
  if (!track && !busy) panel.appendChild(button({ label: "Try Again", onClick: actions.retry }));
  if (state.candidates.length > 0) {
    const list = h("fieldset", { class: "spotify-candidates", attrs: { "aria-label": "YouTube matches" } }, h("legend", { text: "Choose a recording" }));
    for (const candidate of state.candidates) {
      const radio = h("input", { attrs: { type: "radio", name: "spotify-match", value: candidate.videoId, "aria-label": `${candidate.title} — ${candidate.uploader ?? "Unknown channel"}` } });
      radio.checked = state.selectedId === candidate.videoId;
      radio.disabled = busy;
      radio.addEventListener("change", () => actions.select(candidate.videoId));
      const info = h("span", { class: "spotify-candidate-text" },
        h("strong", { text: candidate.title }),
        h("span", { class: "row-hint", text: [candidate.uploader, formatDuration(candidate.duration)].filter(Boolean).join(" · ") }),
        ...candidate.warnings.map((text) => h("span", { class: "row-hint", text })),
      );
      const art = candidate.thumbnail ? h("img", { class: "entry-thumb", attrs: { src: candidate.thumbnail, alt: "", referrerpolicy: "no-referrer", loading: "lazy", draggable: false } }) : null;
      art?.addEventListener("error", () => art.remove());
      list.appendChild(h("label", { class: "spotify-candidate" }, radio, art, info));
    }
    const confirm = button({ label: "Use this match", variant: "primary", onClick: actions.confirm });
    confirm.disabled = busy || !state.selectedId || state.stage === "confirmed";
    append(panel, [list, confirm]);
  } else if (state.stage === "choosing") {
    panel.appendChild(h("p", { attrs: { role: "status" }, text: "No matching recordings found. Try a different artist or title." }));
  }
  if (state.stage === "confirmed") panel.appendChild(h("strong", { text: "Source: YouTube match", attrs: { role: "status" } }));
  return panel;
}
