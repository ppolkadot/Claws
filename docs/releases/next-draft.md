# Next release notes (draft)

Not built or published. Written as the work lands; the version nickname is the owner's choice.

## Removed

- **SteamCMD is gone.** Claws now reads Wallpaper Engine items only from your Steam library, so it needs Steam with Wallpaper Engine and nothing else: no tool to install, no Steam account name, no sign-in. Claws no longer receives a Steam account name at all.
- If you installed SteamCMD through Claws, its folder (and any wallpaper copies SteamCMD saved in it, which can be gigabytes) is still in Claws' tools folder. **Settings → Wallpaper Engine → SteamCMD files from an earlier version** shows its size and **Remove…**. Nothing is deleted until you confirm, and a SteamCMD you chose yourself is never touched. To remove it by hand, close Claws and delete the `steamcmd` folder inside `%APPDATA%\dev.claws.downloader\bin`.
- Settings files from 1.0.9e load unchanged; the old Steam fields are ignored and dropped.

## Added

- **Saving a video wallpaper as MP4 copies the original** when it is 8-bit H.264 in an MP4 (labelled "MP4 (original)"), with a real progress count. The 5.6 GB 4K sample copies in about ten seconds instead of re-encoding for about six minutes. HEVC, VP9, 10-bit and other containers are converted ("MP4 (converted)"); above 1440p the conversion uses a hardware encoder when one works, and keeps the source resolution.
- **GIF is limited**: the first 30 seconds (60 if you choose), at most 720p.
- **The wallpaper card follows Steam**: it shows a download in progress with its percentage, and says when Steam has an update waiting.
- **Scene and web wallpapers can be recorded.** **MP4 (recorded)** has Wallpaper Engine play the wallpaper in a window of its own, hidden off-screen, and records it with Claws' FFmpeg while you keep working: length 10, 30 (default) or 60 seconds, or up to 10 minutes; size from 720p to 4K as the wallpaper was made (never stretched); 30 or 60 fps where the encoder allows. A scene that does not move says so and saves a **Picture (PNG)** instead. Claws runs Wallpaper Engine only after Windows verifies its signature (`Skutta Software GmbH`), and closes its window however the recording ends.
- **Every export shows its progress**: stage, a count, a time left only when it can be backed, "took m:ss" when it ends, on the taskbar button, and in a notification for exports over 30 seconds when Claws is hidden. An estimate shows under the format choice; conversion speeds are remembered.
- A copy or conversion that would run for over ten minutes asks first, and one that would fill the drive stops with a clear message.

## Not in this release

- **Scene sound.** Recordings are silent. A scene's sound is inside its package; adding it is a follow-up.
- Seamless loops, and mouse- or sound-reactive behaviour (recorded idle).
- Unverified: recording while a fullscreen game runs or the screen is locked, a scene's sound playing aloud while recording, behaviour under a heavy GPU load, and AMD or Intel hardware encoders (Claws tests for them and falls back to the next one).
