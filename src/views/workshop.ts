import { api, errorMessage } from "../api";
import { button, toast } from "../ui/components";
import { h } from "../ui/dom";
import { group, row, sectionTitle } from "../ui/layout";

let steamcmdSelected = false;

export function mountWorkshop(url: string): HTMLElement {
  const account = h("input", {
    class: "text-input",
    attrs: { id: "workshop-account", type: "text", maxlength: 64, spellcheck: false, autocomplete: "off" },
  });
  const status = h("div", { class: "footnote", attrs: { role: "status" } });
  const paintDownload = () => {
    download.disabled = choose.disabled || !steamcmdSelected || !account.value.trim();
  };
  account.addEventListener("input", paintDownload);
  const download = button({
    label: "Download ZIP",
    variant: "primary",
    onClick: async () => {
      download.disabled = choose.disabled = account.disabled = true;
      status.textContent = "Downloading from Steam…";
      try {
        const filename = await api.downloadWallpaper(url, account.value);
        const message = `Saved ${filename}.`;
        status.textContent = message;
        if (!status.isConnected) toast(message);
      } catch (error) {
        const message = errorMessage(error);
        status.textContent = message;
        if (!status.isConnected) toast(message, { tone: "error" });
      } finally {
        choose.disabled = account.disabled = false;
        paintDownload();
      }
    },
  });
  download.disabled = true;
  const choose = button({
    label: steamcmdSelected ? "Change SteamCMD" : "Choose SteamCMD",
    variant: "secondary",
    onClick: async () => {
      choose.disabled = true;
      download.disabled = true;
      try {
        if (await api.chooseSteamcmd()) {
          steamcmdSelected = true;
          choose.querySelector("span")!.textContent = "Change SteamCMD";
          status.textContent = "";
        }
      } catch (error) {
        status.textContent = errorMessage(error);
      } finally {
        choose.disabled = false;
        paintDownload();
      }
    },
  });
  const steam = button({
    label: "View in Steam",
    variant: "secondary",
    onClick: () => void api.openWorkshopItem(url).catch((error) => { status.textContent = errorMessage(error); }),
  });
  return h(
    "section", {},
    sectionTitle("Wallpaper Engine"),
    group(
      row("SteamCMD", choose, "Select Valve’s steamcmd.exe from a folder outside OneDrive or other sync folders."),
      row("Steam account name", account, "First sign in through SteamCMD with the account that owns Wallpaper Engine.", account.id),
      row("Workshop item", h("div", { class: "inline" }, steam, download)),
    ),
    status,
  );
}
