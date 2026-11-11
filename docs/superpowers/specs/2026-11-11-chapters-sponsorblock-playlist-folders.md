# Chapters, SponsorBlock and playlist folders (improvement plan phase 5)

**Date:** 2026-11-11 · **Status:** built on `improve/phase-0-safety-net` · **Not in scope:** clip sections and a `claws://` link handler stay deferred.

Three short specifications. Each names what the user gets, what yt-dlp is asked for, what it costs in processing and disk space, and how Fast Save and saved queues treat it.

## 1. Chapters

**Finding.** yt-dlp's `--embed-metadata` already embeds chapters ("Also embeds chapters/infojson if present"). Claws has passed it whenever *Embed metadata* is on, so chapters were already written; nothing said so.

**Change.** `--embed-chapters` is passed explicitly beside `--embed-metadata`, so the behaviour no longer rests on a default of the tool, and Settings and the Fast Save hint name chapters.

- **Option:** none new. Chapters belong to *Embed metadata* (Settings), as the plan grouped them.
- **Processing:** the same FFmpegMetadata pass as before, not another one. `ProcessingPolicy::rewrites_output` is unchanged.
- **Fast Save:** skips it, because it already skips the metadata pass.
- **Not verified:** that a real file carries the chapters (no download with chapters was inspected with ffprobe).

## 2. SponsorBlock

**What it is.** A per-download choice, off by default, for video and audio: **Mark** adds chapter markers for the chosen kinds of segment, **Remove** cuts them out of the file. The kinds are SponsorBlock's categories: sponsor, self-promotion, interaction reminder, intro, outro, preview, non-music section and filler. Sponsor is preselected.

**Where it is offered.** Only for a YouTube link (yt-dlp applies SponsorBlock to YouTube alone), for one video or a playlist, not for a pasted batch or a transcript.

**Network.** yt-dlp asks the SponsorBlock service (`sponsor.ajay.app`) about the video, by a short hash of its ID. This is a destination Claws did not contact before, so the control says so, and nothing is asked unless the user turns it on for that download.

**yt-dlp arguments.**

| Choice | Arguments |
| --- | --- |
| Mark | `--sponsorblock-mark <categories> --embed-chapters` |
| Remove | `--sponsorblock-remove <categories>` |

**Processing and disk.** Both write the finished file again (Mark through the chapter pass, Remove through ModifyChapters), so `ProcessingPolicy::rewrites_output` is true and admission and the size preview leave room for the temporary copy. Remove makes the kept file smaller than the streams downloaded by an amount no one knows in advance: the download size shown is unchanged (those bytes are transferred), and an exact estimate is lowered to approximate.

**Fast Save.** Skips SponsorBlock altogether, as the plan requires for a rewrite pass. The control is disabled and the Fast Save hint names it.

**Saved data.** Two defaulted fields on the download options (`sponsorblock`, `sponsorblockCategories`). Old queues load with it off; queue schema stays 3. The choice is part of the job's frozen options and of the duplicate key.

**Validation.** Categories are checked against the fixed list, sorted and deduplicated. On with no category is refused. Off, a transcript or Fast Save clears the categories.

**Risks.** Cuts are made at keyframes without re-encoding, so a cut can show a brief glitch. The service can be unreachable; yt-dlp then continues without it. Not verified against a real video.

## 3. Playlist into a numbered folder

**What it is.** A per-download switch on a playlist, off by default: the files go into a folder named after the playlist, each prefixed with its position (`My Playlist\03 - Title [id].mp4`).

**Why not yt-dlp's `%(playlist_title)s`.** Claws queues each chosen video as its own job from the video's own link, so yt-dlp has no playlist fields to fill. The folder and number are therefore fixed by Claws when the videos are queued.

**How.**

- The interface sends each item's playlist title, position (its place in the playlist, not among the ticked ones) and the playlist's length.
- The backend builds a prefix, `<folder>/<NN> - `, and stores it on the job. The folder name is made safe for Windows: reserved characters and control characters become spaces, trailing dots and spaces go, a reserved device name gets a leading underscore, an empty result becomes `Playlist`, and it is cut to 80 characters. The number is padded to the playlist's length, at least two digits.
- When the job first starts, the prefix (with `%` escaped) is put in front of the frozen filename template. A resume or retry reuses the frozen template, so the folder never changes under a download.

**Ownership and overwriting.** Unchanged. The file is written inside the job's work folder under the same relative path, and publication moves it to the output folder with the existing rules: the destination folder is created, nothing existing is replaced, and only files the manifest owns are moved. Templates with a folder (`%(uploader)s/...`) already went this way.

**Saved data.** One defaulted option (`playlistFolder`) and one defaulted job field (`namePrefix`). Queue schema stays 3.

**Not in scope.** A batch of pasted links, and choosing the folder's name.
