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

No URL was available to GET. These are recent browse results; an older sample is still owed.

## Installed Steam library

`Get-ItemProperty HKCU:\Software\Valve\Steam` returned no SteamPath, `Get-Command steamcmd.exe` found no installed command, and the default `C:\Program Files (x86)\Steam\steamapps\libraryfolders.vdf` was absent. An installed signed-in Steam library and subscribed project were not available for a live check. Steam could still be installed in another Windows account or location.

## Decision

Keep the current SteamCMD-only source pending an owning-account live test and a subscribed local-library test. The public-URL sample gives no basis to implement that source; the library path is plausible but unproven on this machine. Do not use third-party Workshop mirrors, shared accounts, or content the user's account is not entitled to.

Outstanding: older-item sample, fixture parsing and failure tests, and the local library live check. The generic SteamCMD failure must not be treated as a specific access-denied signal.
