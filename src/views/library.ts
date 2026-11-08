import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorMessage, type HistoryItem } from "../api";
import { formatBytes, formatDate, formatDuration } from "../format";
import { store } from "../store";
import { button, confirmSheet, toast } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { fileDragButton } from "../ui/file-drag";
import type { MountView } from "./shell";

const fail = (error: unknown) => toast(errorMessage(error), { tone: "error" });

function thumbSrc(item: HistoryItem): string | null {
  return item.thumbnailLocal ? convertFileSrc(item.thumbnailLocal) : item.thumbnail;
}

async function confirmRemove(item: HistoryItem) {
  const choice = await confirmSheet(
    "Remove from Library?",
    `“${item.title}” will leave your library. You can also delete the file from your disk.`,
    [
      { label: "Remove from Library", value: "remove", variant: "primary" },
      ...(item.filePath ? [{ label: "Delete File Too", value: "delete", variant: "danger" as const }] : []),
    ],
  );
  if (!choice) return;
  try {
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
      h("span", { class: "tile-sub", text: meta }),
      h("span", { class: "tile-date", text: formatDate(item.finishedAt) }),
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
      const choice = await confirmSheet("Clear the whole library?", "This removes every entry from the library. Downloaded files stay on your disk.", [
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

  const grid = h("ul", { class: "tiles", attrs: { "aria-label": "Downloaded videos" } });
  const note = h("div", { class: "empty" });
  container.append(grid, note);

  // Reused by id so a search keystroke moves cards instead of rebuilding them.
  const built = new Map<string, { key: string; el: HTMLElement }>();
  const tileFor = (item: HistoryItem) => {
    const key = `${item.title}|${item.thumbnailLocal ?? item.thumbnail}|${item.fileSize}`;
    const cached = built.get(item.id);
    if (cached && cached.key === key) return cached.el;
    const el = tile(item);
    built.set(item.id, { key, el });
    return el;
  };

  const render = () => {
    const query = search.value.trim().toLowerCase();
    const items = store.history.filter(
      (i) => !query || i.title.toLowerCase().includes(query) || (i.uploader ?? "").toLowerCase().includes(query),
    );
    const live = new Set(store.history.map((i) => i.id));
    for (const id of built.keys()) if (!live.has(id)) built.delete(id);
    clearAll.disabled = store.history.length === 0;
    searchBox.hidden = store.history.length === 0;
    grid.replaceChildren(...items.map(tileFor));
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
  const off = store.on("history", render);
  render();
  return off;
};
