import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type HistoryItem } from "../api";
import { formatBytes, formatDate, formatDuration } from "../format";
import { keptClearWarning, keptLabel, keptOutcome, keptRemovalWarning, repairNote } from "../kept-files";
import { TileCache, reconcileChildren } from "../library-cache";
import { store } from "../store";
import { button, confirmSheet, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { fileDragButton } from "../ui/file-drag";
import { matchesLibrarySearch } from "../library-search";
import type { MountView } from "./shell";

const fail = (error: unknown) => toast(errorMessage(error), { tone: "error" });

/** Opens the folder of an entry's kept subtitle files and says what happened, success or not. */
export async function showKept(id: string) {
  try {
    const outcome = keptOutcome(await api.showKeptFiles(id));
    toast(outcome.text, { tone: outcome.tone });
  } catch (error) {
    fail(error);
  }
}

function thumbSrc(item: HistoryItem): string | null {
  return item.thumbnailLocal ? convertFileSrc(item.thumbnailLocal) : item.thumbnail;
}

/**
 * An entry can take its file along only when Claws recorded proof that the file at that path
 * is the one it saved. Older entries have none: they can leave the Library, and the file is
 * left for the user to deal with through Show in Folder.
 */
async function confirmRemove(item: HistoryItem) {
  const hasFile = Boolean(item.filePath);
  const provable = hasFile && Boolean(item.fileIdentity);
  const base = provable
    ? `“${item.title}” will leave your library. You can also delete the file from your disk.`
    : hasFile
      ? `“${item.title}” will leave your library. This download was saved by an older version of Claws, which did not record enough to prove the file at that location is still the one it saved, so Claws will not delete it from your disk. Use Show in Folder to find it.`
      : `“${item.title}” will leave your library.`;
  const message = [base, keptRemovalWarning(item)].filter(Boolean).join(" ");
  const choice = await confirmSheet("Remove from Library?", message, [
    { label: "Remove from Library", value: "remove", variant: "primary" },
    ...(provable ? [{ label: "Delete File Too", value: "delete", variant: "danger" as const }] : []),
    ...(hasFile && !provable ? [{ label: "Show in Folder", value: "reveal", variant: "secondary" as const }] : []),
  ]);
  if (!choice) return;
  try {
    if (choice === "reveal") {
      await api.revealDownload(item.id);
      return;
    }
    await api.deleteHistory(item.id, choice === "delete");
    await store.refreshHistory();
  } catch (error) {
    fail(error);
  }
}

function modeIcon(item: HistoryItem) {
  return item.mode === "audio" ? "music" : item.mode === "transcript" ? "captions" : "video";
}

function tile(item: HistoryItem): HTMLElement {
  const art = thumbSrc(item);
  const thumb = art
    ? h("img", { class: "tile-img", attrs: { src: art, alt: "", loading: "lazy", decoding: "async", referrerpolicy: "no-referrer", draggable: false } })
    : h("div", { class: "tile-img thumb-fallback" }, icon(modeIcon(item), 26));
  thumb.addEventListener("error", () => thumb.replaceWith(h("div", { class: "tile-img thumb-fallback" }, icon(modeIcon(item), 26))));
  const duration = formatDuration(item.duration);
  const meta = [item.quality, formatBytes(item.fileSize)].filter(Boolean).join("  ·  ");
  const kept = keptLabel(item);
  return h(
    "li",
    { class: "tile glass" },
    h(
      "div",
      { class: "tile-art" },
      thumb,
      duration ? h("span", { class: "tile-duration", text: duration }) : null,
      h(
        "div",
        { class: "tile-actions" },
        item.filePath ? fileDragButton(item.id) : null,
        button({ icon: "play", title: "Open file", variant: "plain", onClick: () => void api.openDownload(item.id).catch(fail) }),
        button({ icon: "folder", title: "Show in folder", variant: "plain", onClick: () => void api.revealDownload(item.id).catch(fail) }),
        button({ icon: "trash", title: "Remove…", variant: "plain", onClick: () => void confirmRemove(item) }),
      ),
    ),
    h(
      "div",
      { class: "tile-text" },
      h("span", { class: "tile-title", text: item.title, attrs: { title: item.title } }),
      item.uploader ? h("span", { class: "tile-sub", text: item.uploader }) : null,
      item.matchOrigin ? h("span", { class: "tile-sub", text: "Source: YouTube match", attrs: { title: [item.matchOrigin.spotifyTitle, ...item.matchOrigin.spotifyArtists].filter(Boolean).join(" · ") } }) : null,
      h("span", { class: "tile-sub", text: meta }),
      h("span", { class: "tile-date", text: formatDate(item.finishedAt) }),
      // Always visible (the hover actions are not): a kept file must stay findable.
      kept ? h("div", { class: "tile-kept" }, button({ icon: "captions", label: kept, title: "Show kept files", variant: "secondary", size: "small", onClick: () => void showKept(item.id) })) : null,
    ),
  );
}

export const mountLibrary: MountView = (container, ctx) => {
  const search = h("input", {
    class: "search-input",
    attrs: { type: "search", placeholder: "Search", "aria-label": "Search the library", spellcheck: false },
  });
  const searchBox = h("label", { class: "search glass" }, icon("search", 15), search);
  const clearAll = button({
    label: "Clear All",
    variant: "secondary",
    size: "small",
    onClick: async () => {
      const message = ["This removes every entry from the library. Downloaded files stay on your disk.", keptClearWarning(store.history)].filter(Boolean).join(" ");
      const choice = await confirmSheet("Clear the whole library?", message, [
        { label: "Clear Library", value: "clear", variant: "danger" },
      ]);
      if (choice !== "clear") return;
      try {
        await api.clearHistory();
        await store.refreshHistory();
      } catch (error) {
        fail(error);
      }
    },
  });
  ctx.setActions([searchBox, clearAll]);

  // Folders that could not be checked for kept subtitle files of older downloads, with a retry.
  const keptNote = h("div", { class: "kept-note glass", attrs: { role: "status" } });
  keptNote.hidden = true;
  const paintKeptNote = (report: Parameters<typeof repairNote>[0]) => {
    const text = repairNote(report);
    keptNote.hidden = text === null;
    if (text === null) return;
    const retry = button({
      label: "Check Again",
      variant: "secondary",
      size: "small",
      onClick: () => {
        retry.disabled = true;
        void api
          .repairKeptFiles()
          .then((next) => {
            paintKeptNote(next);
            if (next.attached > 0) void store.refreshHistory();
          })
          .catch(fail)
          .finally(() => (retry.disabled = false));
      },
    });
    replace(keptNote, icon("alert", 15), h("span", { class: "kept-note-text", text }), retry);
  };
  // Read again whenever the Library changes: the startup look runs in the background and may
  // finish after this view opened.
  const readReport = () => void api.keptFilesReport().then(paintKeptNote).catch(() => {});
  readReport();

  const grid = h("ul", { class: "tiles", attrs: { "aria-label": "Downloaded videos" } });
  const note = h("div", { class: "empty" });
  container.append(keptNote, grid, note);

  // Reused by id so a search keystroke moves cards instead of rebuilding them. A tile is rebuilt
  // once when a value it shows or acts on changed (library-cache.ts), and never for anything else.
  const tiles = new TileCache<HTMLElement>();
  const tileFor = (item: HistoryItem) => tiles.get(item, () => tile(item));

  const render = () => {
    const query = search.value.trim().toLowerCase();
    const items = store.history.filter(
      (i) => matchesLibrarySearch(i, query),
    );
    tiles.prune(new Set(store.history.map((i) => i.id)));
    clearAll.disabled = store.history.length === 0;
    searchBox.hidden = store.history.length === 0;
    reconcileChildren(grid, items.map(tileFor)); // tiles that stay are left in place, with their focus
    grid.hidden = items.length === 0;
    note.hidden = items.length > 0;
    if (items.length === 0) {
      const none = store.history.length === 0;
      replace(
        note,
        h("div", { class: "empty-symbol" }, icon(none ? "grid" : "search", 46)),
        h("h2", { class: "empty-title", text: none ? "Your library is empty" : "No matches" }),
        h("p", {
          class: "empty-text",
          text: none ? "Finished downloads show up here with their artwork, so you can find and open them again." : `Nothing in your library matches “${search.value.trim()}”.`,
        }),
        none ? button({ label: "Paste a Link", icon: "link", variant: "primary", onClick: () => store.go("download") }) : null,
      );
    }
  };
  let searchTimer: number | undefined;
  search.addEventListener("input", () => {
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(render, 90);
  });
  const offHistory = store.on("history", render);
  const offReport = store.on("history", readReport);
  render();
  return () => {
    offHistory();
    offReport();
  };
};
