# Wallpaper Engine no-sign-in spike

2026-11-25. This is an incomplete spike, not proof that any source works for all items.

## Anonymous SteamCMD

Downloaded Valve's `https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip` into a temporary directory and ran `steamcmd.exe +login anonymous +workshop_download_item 431960 2250845956 +quit`. Anonymous login succeeded, then the command exited 1 with the exact line:

```text
ERROR! Download item 2250845956 failed (Failure).
```

This generic failure does not establish its cause and should not be classified specifically as an entitlement refusal without further evidence.

## Public file URL

Fetched `https://steamcommunity.com/workshop/browse/?appid=431960`, selected ten item IDs from the browse results, then POSTed `itemcount=10` and `publishedfileids[0..9]` to `https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/`. All returned result 1 and consumer app 431960. All had an empty `file_url`, a nonempty `hcontent_file`, and a `file_size`:

| ID | Type | hcontent_file | file_size bytes |
|---|---|---:|---:|
| 3842265910 | Scene | 3687348949913899328 | 66064645 |
| 3843147764 | Scene | 2046093329007459738 | 31793118 |
| 3842760548 | Scene | 3738885625556043809 | 110635340 |
| 3844235110 | Video | 799400690962300731 | 1120439635 |
| 3843547068 | Video | 194917997321324717 | 52926127 |
| 3842247388 | Web | 8026324098842952533 | 127498664 |
| 3843557173 | Video | 7257385576431600555 | 1117462232 |
| 3842750517 | Scene | 4471133486451547573 | 57621190 |
| 3843532597 | Video | 4905918337178760546 | 1119751092 |
| 3845257207 | Video | 2916949187672143975 | 78427040 |

No URL was available to GET.

On 2026-11-25, fetched the Workshop browse page with `browsesort=totaluniquesubscribers&section=readytouseitems`, selected its first ten distinct IDs, and POSTed them in one `GetPublishedFileDetails/v1` request using `itemcount` and `publishedfileids[0..9]`. All returned result 1, consumer app 431960, and an empty `file_url`; therefore there was no URL to GET. The creation timestamps include older projects.

| ID | Created (Unix) | hcontent_file | file_size bytes |
|---|---:|---:|---:|
| 3289988463 | 1721061993 | 5810829448153418240 | 4404844 |
| 819916227 | 1481977526 | 7127631154494237581 | 2223311 |
| 1118136917 | 1503484178 | 7550196524942611714 | 196188340 |
| 884307090 | 1489615396 | 7298294844346496892 | 669459522 |
| 1081733658 | 1500465293 | 1386516767308653904 | 2861727 |
| 2358176341 | 1610358944 | 8199456387164255200 | 100079364 |
| 2447928310 | 1617658152 | 3533512821650877572 | 229421007 |
| 1289832516 | 1517639675 | 6422192688821745518 | 225992811 |
| 1748506393 | 1558513456 | 2086673573327502330 | 1502995 |
| 2887099508 | 1668157370 | 6711647075493397612 | 126761613 |

## Installed Steam library

`Get-ItemProperty HKCU:\Software\Valve\Steam` returned no SteamPath, `Get-Command steamcmd.exe` found no installed command, and the default `C:\Program Files (x86)\Steam\steamapps\libraryfolders.vdf` was absent. An installed signed-in Steam library and subscribed project were not available for a live check. Steam could still be installed in another Windows account or location.

## Decision

Keep SteamCMD and implement the detected local Steam library as an additional source with fixture coverage, pending an owning-account live test and a subscribed local-library test. Neither public-URL sample gives a basis to implement that source; the library path is plausible but unproven on this machine. Do not use third-party Workshop mirrors, shared accounts, or content the user's account is not entitled to.

## Outcome (2026-11-25)

Implemented on `t3/wallpaper-engine-unslop`: the Steam library source (registry `SteamPath`, `libraryfolders.vdf`, `steamapps\workshop\content\431960\<id>` with `project.json`, read only) with fixture tests for the VDF parser and a multi-library tree; SteamCMD stays as the second source. The anonymous `failed (Failure)` line is classified as a generic failure (tested). Still owed: a live check of the library source on a PC with Steam signed in, and an owning-account SteamCMD download.
