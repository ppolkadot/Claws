// The list of a playlist's videos: which ones are ticked, their size estimate and the
// Download button. It edits the selection it is given in place.
import type { DownloadOptions, ProbeResult } from "../api";
import type { EstimateRun } from "../download-estimates";
import { formatDuration, folderName } from "../format";
import { store, type EstimateItem } from "../store";
import { button } from "../ui/components";
import { h, replace } from "../ui/dom";
import { icon } from "../ui/icons";
import { sectionTitle } from "../ui/layout";
import { estimateButton, estimateLine, estimateSettings, paintRun, runKey, thumbnailEl, type LinkChoices } from "./download-shared";

export interface PlaylistDeps {
  run: EstimateRun<EstimateItem>;
  currentOptions(): DownloadOptions;
  startDownload(trigger: HTMLButtonElement): void;
}

export interface PlaylistView {
  el: HTMLElement;
  /** Repaints the size estimate for the current choices and ticks. */
  paintEstimate(): void;
}

export function mountEntries(r: ProbeResult, choice: LinkChoices, deps: PlaylistDeps): PlaylistView {
  const listRun = deps.run;
  const countLabel = h("span", { class: "entries-count" });
  const list = h("div", { class: "entries", attrs: { role: "group", "aria-label": "Videos in this playlist" } });
  // Created with a label: an icon-only button is a fixed 28 px wide.
  const downloadBtn: HTMLButtonElement = button({ label: "Download", icon: "download", variant: "primary", onClick: () => deps.startDownload(downloadBtn) });
  const listLine = estimateLine(() => void listRun.retryFailed(listKey()), "Retry Failed", "Asks again only for the videos whose size could not be estimated. Sizes already read stay. Download is not affected.");
  const listItems = (): EstimateItem[] => {
    const options = deps.currentOptions();
    return r.entries.filter((_, i) => choice.selected.has(i)).map((e) => ({ url: e.url, playlistIndex: e.playlistIndex, options }));
  };
  // Only the choices: the entries are followed by link and position (EstimateRun.follow),
  // so a refreshed list with other videos at the same positions does not keep the old
  // videos' sizes.
  const listKey = () => runKey(deps.currentOptions(), estimateSettings());
  const estimateBtn = estimateButton(listRun, () => listRun.start(listItems(), listKey()));
  const paintEstimate = () => {
    // Other choices, or an estimated entry unticked: the estimate no longer applies (and
    // follow resets it, which repaints). An entry ticked since is named as not included.
    if (!listRun.follow(listKey(), listItems())) return;
    paintRun(listRun, listLine, estimateBtn, choice.selected.size);
  };

  const paintCount = () => {
    paintEstimate();
    countLabel.textContent = `${choice.selected.size} of ${r.entries.length} selected`;
    const label = choice.selected.size === 1 ? "Download 1 Video" : `Download ${choice.selected.size} Videos`;
    replace(downloadBtn, icon("download", 16), h("span", { text: choice.selected.size === 0 ? "Download" : label }));
    downloadBtn.disabled = choice.selected.size === 0;
  };

  const toggleAll = button({
    label: choice.selected.size === r.entries.length ? "Select None" : "Select All",
    variant: "plain",
    size: "small",
    onClick: () => {
      choice.selected = choice.selected.size === r.entries.length ? new Set() : new Set(r.entries.map((_, i) => i));
      toggleAll.querySelector("span")!.textContent = choice.selected.size === r.entries.length ? "Select None" : "Select All";
      list.querySelectorAll<HTMLButtonElement>(".entry").forEach((el, i) => el.setAttribute("aria-checked", String(choice.selected.has(i))));
      paintCount();
    },
  });

  r.entries.forEach((entry, index) => {
    const item = h(
      "button",
      { class: "entry", attrs: { type: "button", role: "checkbox", "aria-checked": String(choice.selected.has(index)) } },
      h("span", { class: "check" }, icon("check", 12)),
      thumbnailEl(entry.thumbnail, "entry-thumb"),
      h(
        "span",
        { class: "entry-text" },
        h("span", { class: "entry-title", text: entry.title }),
        entry.uploader ? h("span", { class: "entry-sub", text: entry.uploader }) : null,
      ),
      h("span", { class: "entry-duration", text: formatDuration(entry.duration) }),
    );
    item.addEventListener("click", () => {
      if (choice.selected.has(index)) choice.selected.delete(index);
      else choice.selected.add(index);
      item.setAttribute("aria-checked", String(choice.selected.has(index)));
      toggleAll.querySelector("span")!.textContent = choice.selected.size === r.entries.length ? "Select None" : "Select All";
      paintCount();
    });
    list.appendChild(item);
  });
  paintCount();
  const el = h(
    "div",
    { class: "entries-block" },
    h("div", { class: "entries-head" }, sectionTitle("Videos"), countLabel, estimateBtn, toggleAll),
    h("div", { class: "glass group" }, list),
    listLine.root,
    h("div", { class: "actions" }, h("span", { class: "save-to", text: `Saves to ${folderName(store.settings.outputDir)}` }), downloadBtn),
  );
  return { el, paintEstimate };
}
