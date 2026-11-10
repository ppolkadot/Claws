import type { SpotifyMatchState } from "../spotify-match.js";
import { formatDuration } from "../format.js";
import { button } from "../ui/components.js";
import { clear, h } from "../ui/dom.js";

export interface SpotifyMatchActions {
  setQuery(query: string): void;
  search(): void;
  select(videoId: string): void;
  confirm(): void;
  retry(): void;
}

/** A mounted matching panel. Its query field lives as long as the panel does. */
export interface SpotifyMatchView {
  el: HTMLElement;
  update(state: SpotifyMatchState): void;
  dispose(): void;
}

/** Trimmed, de-duplicated texts in first-seen order. */
function distinct(texts: readonly (string | null | undefined)[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const text of texts) {
    const trimmed = text?.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      out.push(trimmed);
    }
  }
  return out;
}

function statusText(state: SpotifyMatchState): string {
  switch (state.stage) {
    case "resolving": return "Reading Spotify track…";
    case "searching": return "Finding YouTube recordings…";
    case "probing": return "Checking the selected recording…";
    case "choosing":
      return state.candidates.length > 0
        ? `${state.candidates.length} ${state.candidates.length === 1 ? "recording" : "recordings"} found. Choose a recording.`
        : "No matching recordings found. Try a different artist or title.";
    default: return "";
  }
}

/**
 * Builds the panel once; `update` changes it in place. The query field is never replaced,
 * so focus, the caret and an input-method composition survive every state change. Metadata
 * is always set as text, and choosing a candidate never starts a download.
 */
export function mountSpotifyMatch(actions: SpotifyMatchActions): SpotifyMatchView {
  let composing = false;
  let disposed = false;
  let candidatesKey = "";
  let focusQueryAfterResolution = false;

  const identity = h("div", { class: "spotify-identity group" });
  // The controller validates 300 Unicode code points; native maxlength counts UTF-16 units.
  const query = h("input", { class: "text-input spotify-query", attrs: { id: "spotify-query", type: "text", "aria-label": "Search YouTube", placeholder: "Artist and song title" } });
  const search = button({ label: "Find on YouTube", onClick: () => actions.search() });
  const retry = button({ label: "Try Again", onClick: () => actions.retry() });
  const notices = h("div", { class: "spotify-notices" });
  const status = h("p", { class: "row-hint spotify-status", attrs: { role: "status", "aria-live": "polite" } });
  const candidatesHost = h("div", { class: "spotify-candidates-host" });
  const confirm = button({ label: "Use this match", variant: "primary", onClick: () => actions.confirm() });
  const confirmed = h("strong", { text: "Source: YouTube match", attrs: { role: "status" } });
  const retryHost = h("div", { class: "spotify-match-action" });
  const confirmHost = h("div", { class: "spotify-match-action" });
  const confirmedHost = h("div", { class: "spotify-confirmed-host" });
  const radios: HTMLInputElement[] = [];
  /** Present in the page only while `on`, so it is also absent for assistive technology. */
  const present = (host: HTMLElement, node: HTMLElement, on: boolean) => {
    if (on && node.parentNode !== host) host.appendChild(node);
    else if (!on && node.parentNode === host) host.removeChild(node);
  };

  // Every edit invalidates earlier work at once, composing or not; only the Enter key waits.
  query.addEventListener("compositionstart", () => { composing = true; });
  query.addEventListener("compositionend", () => { composing = false; });
  query.addEventListener("input", () => { if (!disposed) actions.setQuery(query.value); });
  query.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    // Enter that commits an IME candidate is not a request to search.
    if (composing || event.isComposing) return;
    event.preventDefault();
    if (!search.disabled) actions.search();
  });

  const el = h("div", { class: "spotify-match" },
    identity,
    h("p", { class: "row-hint", text: "Find a recording on YouTube, then choose a match. Audio is downloaded from YouTube. The recording may differ from the Spotify version." }),
    h("div", { class: "spotify-search" }, query, search),
    notices, status, retryHost, candidatesHost, confirmHost, confirmedHost,
  );

  function paintIdentity(state: SpotifyMatchState) {
    const track = state.track;
    clear(identity);
    if (track?.artwork) {
      const image = h("img", { class: "spotify-art", attrs: { src: track.artwork, alt: "", referrerpolicy: "no-referrer", draggable: false } });
      image.addEventListener("error", () => image.remove());
      identity.appendChild(image);
    }
    identity.appendChild(h("span", { class: "row-hint", text: "Spotify track" }));
    identity.appendChild(h("strong", { text: track?.title ?? "Identify this song" }));
    identity.appendChild(h("span", { class: "row-hint", text: [track?.artists.join(", "), formatDuration(track?.duration ?? null)].filter(Boolean).join(" · ") }));
  }

  function paintCandidates(state: SpotifyMatchState, busy: boolean) {
    const key = JSON.stringify(state.candidates);
    if (key !== candidatesKey) {
      candidatesKey = key;
      clear(candidatesHost);
      radios.length = 0;
      if (state.candidates.length > 0) {
        const list = h("fieldset", { class: "spotify-candidates", attrs: { "aria-label": "YouTube matches" } }, h("legend", { text: "Choose a recording" }));
        for (const candidate of state.candidates) {
          const radio = h("input", { attrs: { type: "radio", name: "spotify-match", value: candidate.videoId, "aria-label": `${candidate.title} — ${candidate.uploader ?? "Unknown channel"}` } });
          radio.addEventListener("change", () => actions.select(candidate.videoId));
          radios.push(radio);
          const info = h("span", { class: "spotify-candidate-text" },
            h("strong", { text: candidate.title }),
            h("span", { class: "row-hint", text: [candidate.uploader, formatDuration(candidate.duration)].filter(Boolean).join(" · ") }),
            ...candidate.warnings.map((text) => h("span", { class: "row-hint", text })),
          );
          const art = candidate.thumbnail ? h("img", { class: "entry-thumb", attrs: { src: candidate.thumbnail, alt: "", referrerpolicy: "no-referrer", loading: "lazy", draggable: false } }) : null;
          art?.addEventListener("error", () => art.remove());
          list.appendChild(h("label", { class: "spotify-candidate" }, radio, art, info));
        }
        candidatesHost.appendChild(list);
      }
    }
    for (const radio of radios) {
      radio.checked = state.selectedId === radio.value;
      radio.disabled = busy;
    }
  }

  function update(state: SpotifyMatchState) {
    if (disposed) return;
    const previousFocus = document.activeElement;
    const ownedFocus = previousFocus !== null && el.contains(previousFocus);
    const busy = state.stage === "resolving" || state.stage === "searching" || state.stage === "probing";
    if (focusQueryAfterResolution && previousFocus !== status) focusQueryAfterResolution = false;
    paintIdentity(state);
    // Never write the field while it is being edited: the user's text is the truth.
    if (!composing && query.value !== state.query) query.value = state.query;
    query.disabled = state.stage === "resolving";
    search.disabled = busy || !state.track || !state.query.trim();
    clear(notices);
    const error = state.error?.trim() || null;
    if (error) notices.appendChild(h("div", { class: "notice notice-error", text: error, attrs: { role: "alert" } }));
    for (const warning of distinct(state.warnings)) {
      if (warning !== error) notices.appendChild(h("p", { class: "row-hint", text: warning }));
    }
    status.textContent = statusText(state);
    status.className = `row-hint spotify-status${status.textContent ? "" : " visually-hidden"}`;
    present(retryHost, retry, !busy && state.retryable);
    retry.disabled = false;
    paintCandidates(state, busy);
    present(confirmHost, confirm, state.candidates.length > 0);
    confirm.disabled = busy || !state.selectedId || state.stage === "confirmed";
    present(confirmedHost, confirmed, state.stage === "confirmed");
    for (const host of [notices, retryHost, candidatesHost, confirmHost, confirmedHost]) host.hidden = !host.firstChild;
    if (previousFocus === confirm && state.stage === "probing") {
      // Disabling the focused confirmation must not drop keyboard focus out of the stage.
      status.tabIndex = -1;
      status.focus({ preventScroll: true });
    } else if (ownedFocus && previousFocus && !el.contains(previousFocus)) {
      if (!query.disabled) query.focus({ preventScroll: true });
      else { focusQueryAfterResolution = true; status.tabIndex = -1; status.focus({ preventScroll: true }); }
    } else if (focusQueryAfterResolution && !query.disabled) {
      focusQueryAfterResolution = false;
      query.focus({ preventScroll: true });
    }
  }

  return { el, update, dispose() { disposed = true; } };
}

/** One-shot render, for callers (and tests) that do not keep the panel. */
export function renderSpotifyMatch(state: SpotifyMatchState, actions: SpotifyMatchActions): HTMLElement {
  const view = mountSpotifyMatch(actions);
  view.update(state);
  return view.el;
}
