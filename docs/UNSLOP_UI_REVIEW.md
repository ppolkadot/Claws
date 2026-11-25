# Unslop UI Review and Implementation Plan

- **Date:** 2026-11-23
- **Repository:** `C:\Users\A3\Claws` (v1.0.9)
- **Skill Applied:** `unslop-ui` v1.0.1 (`github.com/felipebasurto/unslop-ui/tree/main/skills/unslop-ui`: `SKILL.md`, `references/surface.md`, `references/examples.md`, `references/taste.md`)
- **Target Scope:** All frontend views, UI primitives, styles, icons, and pure-logic copy modules under [src/](file:///C:/Users/A3/Claws/src) and [index.html](file:///C:/Users/A3/Claws/index.html), evaluated with both copy/decoration rules (`U1`–`U16`, `U21`–`U24`, `unslop` writing rules, `length`) and `--surface` visual rules (`U17`–`U20`, `U25`).
- **Taste File (`.unslop-ui/taste.md`):** Not present (`Off by taste file: none`).
- **Contract References Checked:** [docs/HANDOFF.md](file:///C:/Users/A3/Claws/docs/HANDOFF.md), [PRODUCT.md](file:///C:/Users/A3/Claws/PRODUCT.md), [DESIGN.md](file:///C:/Users/A3/Claws/DESIGN.md), [docs/CURRENT_DEVELOPMENT_STATUS.md](file:///C:/Users/A3/Claws/docs/CURRENT_DEVELOPMENT_STATUS.md).

---

## Verdict: Light

Claws is already a restrained, utility-first desktop interface with no hero slogans (`U8`), no decorative `01/02/03` numbering (`U2`), no fake live badges (`U9`), no arrow glyphs on links or buttons (`U21`), no generic `"Learn more"` / `"Click here"` actions (`U22`), no charts or stat grids (`U12`–`U16`, `U23`), and no single-edge accent stripes (`U24`).

The remaining tells fall into four focused groups:

1. **Title Case on buttons, actions, and one section heading (`unslop 17`)** across 11 view files, inconsistent with existing sentence-case actions (`"Move to top"`, `"Try again"`, `"Show in folder"`, `"Use this match"`).
2. **Self-restating, passive, or punctuation-heavy helper copy (`U3`, `U4`, `U5`, `U7`, `unslop 13, 14, 28, 29, 30`)** in Download, Options, Library, and Settings.
3. **Static green status dots and redundant button icons (`U10`, `U11`, `U14`)** in Settings, Setup, Batch, Queue, Library, plus three unused icon definitions in [src/ui/icons.ts](file:///C:/Users/A3/Claws/src/ui/icons.ts).
4. **Surface spacing and edge-alignment drift (`U18`, `U25`)** in [src/styles/base.css](file:///C:/Users/A3/Claws/src/styles/base.css), [src/styles/components.css](file:///C:/Users/A3/Claws/src/styles/components.css), and [src/styles/views.css](file:///C:/Users/A3/Claws/src/styles/views.css).

### Summary Table

| Rule | Found | Planned Fix | Kept | Notes |
| --- | ---: | ---: | ---: | --- |
| `U1` eyebrow label | 1 | 0 | 1 | `"Spotify track"` kept for provenance ([HANDOFF.md:38](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L38)) |
| `U2` decorative numbering | 0 | 0 | 0 | — |
| `U3` subtitle reflex | 2 | 2 | — | `settings.ts:171, 401`; technical helper modules kept |
| `U4` botsplaining | 9 | 9 | — | Empty states & self-describing hints tightened; `download.ts:871` also logged as task |
| `U5` leaked design notes | 1 | 1 | 0 | `download-options.ts:161` removes `yt-dlp` subprocess detail while keeping `sponsor.ajay.app` |
| `U6` status strings (`·`-joined) | 13 | 0 | 13 | All bound to live telemetry, probed metadata, or provenance |
| `U7` labeling everything | 8 | 7 | 0 | `1` logged as owner task (`settings.ts:97-114` `<label for>` association) |
| `U8` staccato / slogans | 0 | 0 | 0 | — |
| `U9` badges without real state | 2 | 0 | 2 | Queue active count & Library duration badge |
| `U10` status dots | 4 | 2 | 2 | Static green dots removed (`settings.ts:168, 368`); live tool dots kept |
| `U11` decorative emojis/icons | 10 | 10 | 0 | 7 redundant text-button icons + 3 unused icon definitions (`icons.ts:20, 23, 26`) |
| `U12–U16`, `U23` charts / KPIs | 0 | 0 | 0 | No charts or stat grids in Claws |
| `U17` gradients & glows | 0 | 0 | 0 | Functional neutral scrim & liquid rim follow [DESIGN.md](file:///C:/Users/A3/Claws/DESIGN.md) |
| `U18` off-scale spacing | 17 | 17 | 0 | Normalized to `4/6/8/12/16/20/28` (and `2/6/10/14` inside controls) |
| `U19` extra-heavy weights (800/900) | 0 | 0 | 0 | Max CSS font weight is `700` |
| `U20` decorative motion | 1 | 0 | 0 | Spring system kept ([motion.ts](file:///C:/Users/A3/Claws/src/ui/motion.ts)); `.tile:hover` logged as owner task |
| `U21` arrow on links/buttons | 0 | 0 | 0 | — |
| `U22` generic link/button names | 0 | 0 | 0 | — |
| `U24` edge accent stripe | 0 | 0 | 0 | — |
| `U25` edges that don't agree | 4 | 4 | 0 | `.entries-head`, `.large-title`, `.queue-summary`, `.spotify-candidate` |
| `length` (≤ 5 words) | 0 | 0 | 0 | All section headings, row titles, and short captions already ≤ 5 words |
| `unslop 13` em dashes | 1 | 1 | 0 | `download-shared.ts:79` (`document.title` / `aria-label` excluded as non-prose) |
| `unslop 14` mid-sentence colons | 2 | 2 | 0 | `download-options.ts:60, 161` |
| `unslop 17` title case | 46 | 46 | 0 | 46 locations (58 Title Case strings across 11 view files) |
| `unslop 28` dense sentences | 4 | 4 | 0 | `download-options.ts:76`, `library.ts:244`, `settings.ts:124, 134` |
| `unslop 29` passive voice | 9 | 9 | 0 | 9 locations (11 strings) converted to active voice |
| `unslop 30` adverbs | 1 | 1 | 0 | `settings.ts:129` (`"substantially"`) |

> Note: Totals above count distinct locations per rule; items tagged with multiple rules (such as `U4` + `U7` or `unslop 14` + `unslop 29`) appear under each applicable rule.

---

## 2. Rule-by-Rule Evaluation Across `U1–U25` and Writing Rules

### Rules with Findings or Kept Uses
- **`U1` Eyebrow labels:** Only one small uppercase-adjacent label exists (`"Spotify track"` at [src/views/spotify-match.ts:100](file:///C:/Users/A3/Claws/src/views/spotify-match.ts#L100)). Kept because [docs/HANDOFF.md:38](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L38) requires separate provenance distinguishing Spotify identity from YouTube candidates.
- **`U3` Subtitle reflex:** Two explanatory lines repeat what is already visible (`"In use throughout Claws."` in [settings.ts:171](file:///C:/Users/A3/Claws/src/views/settings.ts#L171) and the first sentence of the Tools footnote in [settings.ts:401](file:///C:/Users/A3/Claws/src/views/settings.ts#L401)). All remaining subtitles/hints on Settings and Download rows convey non-obvious behaviors or technical trade-offs (codecs, remuxing, cookies, SponsorBlock network behavior, bitrate).
- **`U4` Botsplaining:** Nine items either narrate what the UI element visibly does or pad empty states with extra motivation.
- **`U5` Leaked design notes:** One hint exposes backend tool implementation (`"yt-dlp asks sponsor.ajay.app"` in [download-options.ts:161](file:///C:/Users/A3/Claws/src/views/download-options.ts#L161)).
- **`U6` Status strings:** Thirteen `·`-joined lines across Download, Batch, Queue, Library, and Spotify match views; every one carries live download telemetry, probed media metadata, or required source attribution.
- **`U7` Labeling everything:** Seven cases repeat an adjacent label, `<optgroup>`, or runtime name (`download-options.ts:63, 76, 191`, `settings.ts:147, 223, 256, 361-370`); one `<input>` (`settings.ts:97-114`) relies on `aria-label` instead of `<label for>` and is logged as an owner task.
- **`U9` Badges without real state:** Two badges (`shell.ts:60, 195-200` active queue count; `library.ts:104` duration overlay); both render from live data.
- **`U10` Status dots:** Two static green dots (`settings.ts:168, 368`) fail the real-state test and are planned for removal; two live tri-state dots (`shell.ts:75, 210`; `setup-panel.ts:57`) render dynamically from tool status and are kept.
- **`U11` Decorative emojis/icons:** No emojis exist in UI copy. Seven text-labeled buttons carry redundant icons that their sibling buttons omit (`download-batch.ts:96`, `setup-panel.ts:71`, `settings.ts:340, 356, 375`, `queue.ts:289`, `library.ts:249`), and three SVG icon definitions in [icons.ts:20, 23, 26](file:///C:/Users/A3/Claws/src/ui/icons.ts#L20-L26) (`open`, `clock`, `sparkles`) are unreferenced across `src/`.
- **`U18` Off-scale spacing (`--surface`):** Seventeen off-scale `gap`, `margin`, and `padding` values (`1px`, `5px`, `7px`, `9px`, `11px`, `13px`, `18px`, `34px`) appear in [base.css](file:///C:/Users/A3/Claws/src/styles/base.css), [components.css](file:///C:/Users/A3/Claws/src/styles/components.css), and [views.css](file:///C:/Users/A3/Claws/src/styles/views.css). Note that `26px` top margin on `.section-title` ([views.css:45](file:///C:/Users/A3/Claws/src/styles/views.css#L45)) is explicitly recorded in [DESIGN.md:264](file:///C:/Users/A3/Claws/DESIGN.md#L264) (`"Section titles sit 26px above their group."`) and is treated as a design-policy target per `surface.md`.
- **`U20` Decorative motion (`--surface`):** Claws's sampled spring system ([motion.ts](file:///C:/Users/A3/Claws/src/ui/motion.ts)) is documented in [DESIGN.md:346-379](file:///C:/Users/A3/Claws/DESIGN.md#L346-L379) and verified by `npm run test:motion`. Library tile hover elevation (`translateY(-2px)` + `scale(1.03)` on `.tile-img` at [views.css:862-880](file:///C:/Users/A3/Claws/src/styles/views.css#L862-L880)) is flagged as an owner task because it matches `U20`'s `hover:scale-*` trigger while also appearing in [DESIGN.md:272, 341](file:///C:/Users/A3/Claws/DESIGN.md#L272).
- **`U25` Edges that don't agree (`--surface`):** Four edge/bar misalignments (`.entries-head` baseline/center mismatch vs `.recent-head`; `.large-title` and `.queue-summary` left offsets vs `6px`; `.spotify-candidate` horizontal padding `8px` vs `16px`).
- **`unslop 13, 14, 17, 28, 29, 30`:** One em dash (`download-shared.ts:79`), two mid-sentence colons (`download-options.ts:60, 161`), 46 Title Case locations (58 strings), four dense sentences, nine passive-voice locations (11 strings), and one unnecessary adverb (`"substantially"`).

### Rules Checked with Zero Violations
- **`U2` Decorative numbering:** No `01/02/03`, `§`, or `//` prefixes.
- **`U8` Staccato / slogans:** No marketing fragments or taglines.
- **`U12`–`U16` & `U23` Charts & stat cards:** No charts, KPI grids, legends, or stat-card rows exist.
- **`U17` Gradients and glows:** Panels and buttons use neutral tints, solid `--blue-fill` / `#d92d20`, and soft cool-black drop shadows ([DESIGN.md:270-279](file:///C:/Users/A3/Claws/DESIGN.md#L270-L279)). `.tile-actions` ([views.css:904](file:///C:/Users/A3/Claws/src/styles/views.css#L904)) uses a neutral `transparent -> rgba(0,0,0,0.55)` image scrim for white icon legibility, and `.glass::before` ([components.css:17](file:///C:/Users/A3/Claws/src/styles/components.css#L17)) uses a single-neutral specular rim specified by [DESIGN.md:204](file:///C:/Users/A3/Claws/DESIGN.md#L204).
- **`U19` Extra-heavy weights:** Maximum font weight in CSS is `700` (`.large-title`, `.preview-title`, `.empty-title`, `.sheet-title`).
- **`U21` Arrows on links/buttons:** No `→` or trailing `<ArrowRight>` glyphs on buttons or links.
- **`U22` Generic link/button names:** No `"Click here"`, `"Learn more"`, `"Read more"`, or `"Submit"`.
- **`U24` Edge accent stripes:** No `border-left` / `::before` / `inset` color stripes on cards or rows.
- **`length`:** All section headings (`"Download"`, `"Queue"`, `"Library"`, `"Settings"`, `"Options"`, `"Videos"`, `"Links"`, `"Recent"`, `"Downloads"`, `"Quick access"`, `"Appearance"`, `"Sign-in"`, `"Tools"`) and row labels are 1–4 words (≤ 5 words).
- **Other `unslop` writing rules (`3, 5, 7–12, 16, 23, 25–27, 31–32`):** No superficial `-ing` phrases, vague attributions, AI buzzwords (`crucial`, `delve`, `seamless`, `leverage`, etc.), bold lead-ins, abstract metaphors, or filler phrases (`"in order to"`, `"it is important to note"`). (`"over a thousand sites"` in `download.ts:871` is handled under the unverified-claim guardrail and `U4`.)

---

## 3. Detailed Findings — Copy, Punctuation & Helper Text

1. **[src/views/download.ts:868](file:///C:/Users/A3/Claws/src/views/download.ts#L868) — `U4`**
   - **Why:** `"to get started"` is onboarding filler in the empty-state heading.
   - **Current:** `"Paste a link to get started"`
   - **Proposed:** `"Paste a link"`

2. **[src/views/download.ts:871](file:///C:/Users/A3/Claws/src/views/download.ts#L871) — `U4` (and unverified-claim guardrail; see Section 8)**
   - **Why:** `"Claws saves video and audio from over a thousand sites."` makes a site-count promise ([PRODUCT.md:43](file:///C:/Users/A3/Claws/PRODUCT.md#L43) cautions against promising extraction beyond yt-dlp, while [PRODUCT.md:23](file:///C:/Users/A3/Claws/PRODUCT.md#L23) notes yt-dlp's 1,800+ extractors) and pads the actionable instruction.
   - **Current:** `"Claws saves video and audio from over a thousand sites. Copy a link, then press Ctrl+V anywhere in this window."`
   - **Proposed:** `"Copy a link, then press Ctrl+V anywhere in this window."`

3. **[src/views/download-shared.ts:79](file:///C:/Users/A3/Claws/src/views/download-shared.ts#L79) — `unslop 13`**
   - **Why:** Uses an em dash (`—`) in `TEXT_ONLY`, which is rendered directly in the estimate line (`paintPreviewEstimate` at [download.ts:515](file:///C:/Users/A3/Claws/src/views/download.ts#L515)).
   - **Current:** `export const TEXT_ONLY = "Text only — the video is not downloaded";`
   - **Proposed:** `export const TEXT_ONLY = "Text only, the video is not downloaded";`

4. **[src/views/download-options.ts:60](file:///C:/Users/A3/Claws/src/views/download-options.ts#L60) — `unslop 14, 29`**
   - **Why:** Joins two independent clauses with a mid-sentence colon (`:`) and uses passive voice (`"are made"`).
   - **Current:** `"Transcripts are made one video at a time: open a video's own link to save its captions."`
   - **Proposed:** `"Transcripts work one video at a time. Open a single video link to save its captions."`

5. **[src/views/download-options.ts:63](file:///C:/Users/A3/Claws/src/views/download-options.ts#L63) — `U4, U7`**
   - **Why:** When `Transcript` is disabled in the Type segmented control, appending `", so there is no transcript to save"` restates the disabled state.
   - **Current:** `` `${NO_CAPTIONS}, so there is no transcript to save.` ``
   - **Proposed:** `` `${NO_CAPTIONS}.` ``

6. **[src/views/download-options.ts:76](file:///C:/Users/A3/Claws/src/views/download-options.ts#L76) — `U7, unslop 28, 29`**
   - **Why:** Uses passive voice (`"Made from"`, `"are generated by the site"`), repeats `"captions"` densely, and keeps the real distinction (captions vs speech recognition, and auto-caption accuracy) wordier than needed.
   - **Current:** `"Made from the site's own captions; Claws does not recognise speech. Automatic captions are generated by the site and can contain errors."`
   - **Proposed:** `"Uses the site's captions rather than speech recognition. Automatic captions can contain errors."`

7. **[src/views/download-options.ts:98](file:///C:/Users/A3/Claws/src/views/download-options.ts#L98) & [src/views/download-batch.ts:273](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L273) — `unslop 29`**
   - **Why:** Passive construction (`"is saved"`, `"Nothing is upscaled"`) in both playlist and batch Quality hints.
   - **Current:** `"Each video is saved at the largest size it has, up to this limit. Nothing is upscaled."`
   - **Proposed:** `"Saves each video at its largest size up to this limit, without upscaling."`

8. **[src/views/download-options.ts:100](file:///C:/Users/A3/Claws/src/views/download-options.ts#L100) — `U4`**
   - **Why:** `"The sizes this video is offered in"` narrates the visible size chips on screen. Only `"by the shorter side"` and `"without upscaling"` add non-obvious facts.
   - **Current:** `"The sizes this video is offered in, by the shorter side. Nothing is upscaled."`
   - **Proposed:** `"Measured by the shorter side, without upscaling."`

9. **[src/views/download-options.ts:101](file:///C:/Users/A3/Claws/src/views/download-options.ts#L101) — `unslop 29`**
   - **Why:** Passive phrasing (`"Saved at"`, `"Nothing is upscaled"`).
   - **Current:** `"Saved at the largest size this video has, up to this limit on its shorter side. Nothing is upscaled."`
   - **Proposed:** `"Saves the largest size available up to this limit on the shorter side, without upscaling."`

10. **[src/views/download-options.ts:161](file:///C:/Users/A3/Claws/src/views/download-options.ts#L161) — `U5, unslop 14`**
    - **Why:** Joins two clauses with a mid-sentence colon and leaks the `yt-dlp` subprocess detail. **Contract preserved:** [docs/HANDOFF.md:31](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L31) requires the UI to state that SponsorBlock contacts `sponsor.ajay.app`.
    - **Current:** `"Uses the community SponsorBlock database: yt-dlp asks sponsor.ajay.app about this video."`
    - **Proposed:** `"Asks sponsor.ajay.app for community-marked segments in this video."`

11. **[src/views/download-options.ts:164-166](file:///C:/Users/A3/Claws/src/views/download-options.ts#L164-L166) — `unslop 29`**
    - **Why:** All three SponsorBlock mode hints (`off`, `mark`, `remove`) use passive voice (`"Nothing is asked"`, `"nothing is cut"`, `"are cut out"`).
    - **Current:**
      - `off`: `` `${about} Nothing is asked while this is off.` ``
      - `mark`: `` `${about} The chosen segments become chapter markers; nothing is cut.` ``
      - `remove`: `` `${about} The chosen segments are cut out of the file. A cut can leave a brief glitch, and the saved file is smaller than the size shown.` ``
    - **Proposed:**
      - `off`: `` `${about} Makes no request while off.` ``
      - `mark`: `` `${about} Marks chosen segments as chapters without cutting.` ``
      - `remove`: `` `${about} Cuts chosen segments from the file, which can leave a brief glitch and reduce the final size.` ``

12. **[src/views/download-options.ts:191](file:///C:/Users/A3/Claws/src/views/download-options.ts#L191) — `U4, U7`**
    - **Why:** Repeats the `"Segments"` row title and the `"Mark"` / `"Remove"` mode selected immediately above it.
    - **Current:** `rows.push(row("Segments", chips, "The kinds of segment to mark or remove."));`
    - **Proposed:** `rows.push(row("Segments", chips));`

13. **[src/views/download-batch.ts:323](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L323) — `unslop 29`**
    - **Why:** Passive construction (`"can be chosen when a link is read on its own"`).
    - **Current:** `"Applies to every link here. Subtitles and transcripts can be chosen when a link is read on its own."`
    - **Proposed:** `"Applies to every link in this list. Open a link on its own for subtitles or a transcript."`

14. **[src/views/library.ts:244](file:///C:/Users/A3/Claws/src/views/library.ts#L244) — `U4, unslop 28`**
    - **Why:** `"with their artwork, so you can find and open them again"` explains what the Library view obviously does.
    - **Current:** `"Finished downloads show up here with their artwork, so you can find and open them again."`
    - **Proposed:** `"Finished downloads appear here."`

15. **[src/views/settings.ts:124](file:///C:/Users/A3/Claws/src/views/settings.ts#L124) — `unslop 28`**
    - **Why:** Dense multi-clause hint; **contract preserved:** [docs/HANDOFF.md:26](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L26) requires explaining per-download scope, next-start timing, and single-fragment/segment execution.
    - **Current:** `"For each download, so several at once can add up to more. Applies from the next download that starts or resumes; a limited download fetches one segment at a time."`
    - **Proposed:** `"Applies per download from the next start or resume, using one segment at a time."`

16. **[src/views/settings.ts:129](file:///C:/Users/A3/Claws/src/views/settings.ts#L129) — `unslop 29, 30`**
    - **Why:** Uses unnecessary adverb `"substantially"` (`unslop 30`) and passive `"can be rewritten"` (`unslop 29`).
    - **Current:** `"Adds title, uploader, date and chapters. Large files may take substantially longer because the whole file can be rewritten."`
    - **Proposed:** `"Adds title, uploader, date and chapters. Rewrites the file after download, which takes longer on large files."`

17. **[src/views/settings.ts:134](file:///C:/Users/A3/Claws/src/views/settings.ts#L134) — `unslop 28, 29`**
    - **Why:** Uses passive `"can be saved"` and wordy `"downloads that have not started yet"`.
    - **Current:** `"For audio downloads, picks a stream already in the chosen format so it can be saved without conversion. Its bitrate may differ from the site's best stream. Applies to downloads that have not started yet."`
    - **Proposed:** `"For audio downloads, saves a stream already in the chosen format without conversion. Bitrate may differ from the site's best stream. Applies to new downloads."`

18. **[src/views/settings.ts:147](file:///C:/Users/A3/Claws/src/views/settings.ts#L147) — `U7`**
    - **Why:** Repeats the row label `"Notify when a download finishes"` instead of stating only the additional conditions (`"also on failure"`, `"only while hidden, minimized or behind another window"`, matching [docs/HANDOFF.md:30](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L30)).
    - **Current:** `"A Windows notification when a download finishes or fails while Claws is hidden or behind another window."`
    - **Proposed:** `"Also notifies on failure. Sent only while Claws is hidden, minimized or behind another window."`

19. **[src/views/settings.ts:171](file:///C:/Users/A3/Claws/src/views/settings.ts#L171) — `U3, U4`**
    - **Why:** `"In use throughout Claws."` restates the loaded font status in `fontStatusRow(true)`. Keep the actionable hint when `!inUse`.
    - **Current:** `inUse ? "In use throughout Claws." : "Salted Flutters Polaris is unavailable. Reinstall Claws to restore the bundled font."`
    - **Proposed:** `inUse ? undefined : "Salted Flutters Polaris is unavailable. Reinstall Claws to restore the bundled font."`

20. **[src/views/settings.ts:223](file:///C:/Users/A3/Claws/src/views/settings.ts#L223) — `U4, U7`**
    - **Why:** `"How much of the desktop shows through."` merely defines the word `"Transparency"` in the row title. The actionable second sentence stays.
    - **Current:** `"How much of the desktop shows through. Lower it if text is hard to read over a busy wallpaper."`
    - **Proposed:** `"Lower this if text is hard to read over a busy wallpaper."`

21. **[src/views/settings.ts:256](file:///C:/Users/A3/Claws/src/views/settings.ts#L256) — `U4, U7`**
    - **Why:** Both branches repeat the `"Cookies file"` label, the `"No file chosen"` / path display, and the `"Choose…"` button.
    - **Current:** `path ? "A cookies.txt exported from your browser." : "Choose a cookies.txt file to start using it."`
    - **Proposed:** Delete the hint argument (`undefined`).

22. **[src/views/settings.ts:401](file:///C:/Users/A3/Claws/src/views/settings.ts#L401) — `U3`**
    - **Why:** `"Claws uses yt-dlp and ffmpeg and runs entirely on this PC."` summarizes the `"Tools"` card immediately above it. Keep only the legal/rights disclaimer.
    - **Current:** `"Claws uses yt-dlp and ffmpeg and runs entirely on this PC. Only download content you have the right to save."`
    - **Proposed:** `"Only download content you have the right to save."`

---

## 4. Detailed Findings — Title Case vs. Sentence Case (`unslop 17`)

Claws already uses sentence case in newer queue and Spotify-match controls ([queue.ts:219, 232, 236](file:///C:/Users/A3/Claws/src/views/queue.ts#L219-L236): `"Move to top"`, `"Try again"`, `"Show in folder"`; [spotify-match.ts:62](file:///C:/Users/A3/Claws/src/views/spotify-match.ts#L62): `"Use this match"`). The 46 locations (58 strings) below convert the remaining Title Case buttons, chips, dialog titles, and section headings to sentence case (preserving proper nouns/brands like `"Claws"`, `"Library"`, `"Ctrl+Alt+S"`, `"yt-dlp"`, `"Spotify"`, `"YouTube"` where part of a specific phrase or shortcut, and following `unslop 17` for UI actions and section headers):

| File & Line | Current | Proposed |
| --- | --- | --- |
| [src/views/download.ts:47](file:///C:/Users/A3/Claws/src/views/download.ts#L47) | `"Open Folder"` | `"Open folder"` |
| [src/views/download.ts:140](file:///C:/Users/A3/Claws/src/views/download.ts#L140) | `"Get Info"` | `"Get info"` |
| [src/views/download.ts:173](file:///C:/Users/A3/Claws/src/views/download.ts#L173) | `` `Look Up ${count} Links` : "Get Info" `` | `` `Look up ${count} links` : "Get info" `` |
| [src/views/download.ts:378](file:///C:/Users/A3/Claws/src/views/download.ts#L378) | `"…Press Set Up Everything above, then try again."` | `"…Press Set up everything above, then try again."` |
| [src/views/download.ts:450](file:///C:/Users/A3/Claws/src/views/download.ts#L450) | `"…Press Set Up Everything above, then try again."` | `"…Press Set up everything above, then try again."` |
| [src/views/download.ts:592](file:///C:/Users/A3/Claws/src/views/download.ts#L592) | `` `${jobs.length} Added` `` | `` `${jobs.length} added` `` |
| [src/views/download.ts:596](file:///C:/Users/A3/Claws/src/views/download.ts#L596) | `"View Queue"` | `"View queue"` |
| [src/views/download.ts:703](file:///C:/Users/A3/Claws/src/views/download.ts#L703) | `"Save Transcript"` | `"Save transcript"` |
| [src/views/download.ts:720](file:///C:/Users/A3/Claws/src/views/download.ts#L720) | `"Show Library"` | `"Show library"` |
| [src/views/download.ts:758](file:///C:/Users/A3/Claws/src/views/download.ts#L758) | `"Back to Links"` | `"Back to links"` |
| [src/views/download.ts:831](file:///C:/Users/A3/Claws/src/views/download.ts#L831) | `"Try Again"` | `"Try again"` |
| [src/views/download-options.ts:32](file:///C:/Users/A3/Claws/src/views/download-options.ts#L32) | `"Audio Only"` | `"Audio only"` |
| [src/views/download-batch.ts:65](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L65) | `"View Queue"` | `"View queue"` |
| [src/views/download-batch.ts:102](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L102) | `"Select All"` | `"Select all"` |
| [src/views/download-batch.ts:119](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L119) | `"Retry Failed"` | `"Retry failed"` |
| [src/views/download-batch.ts:178](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L178) | `` `Retry ${c.failed} Failed` : `Read ${again} Again` `` | `` `Retry ${c.failed} failed` : `Read ${again} again` `` |
| [src/views/download-batch.ts:179](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L179) | `"Select None" : "Select All"` | `"Select none" : "Select all"` |
| [src/views/download-batch.ts:182](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L182) | `` "Download 1 Video" : `Download ${n} Videos` `` | `` "Download 1 video" : `Download ${n} videos` `` |
| [src/views/download-batch.ts:296](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L296) | `"Audio Only"` | `"Audio only"` |
| [src/views/download-playlist.ts:28](file:///C:/Users/A3/Claws/src/views/download-playlist.ts#L28) | `"Retry Failed"` | `"Retry failed"` |
| [src/views/download-playlist.ts:48](file:///C:/Users/A3/Claws/src/views/download-playlist.ts#L48) | `` "Download 1 Video" : `Download ${choice.selected.size} Videos` `` | `` "Download 1 video" : `Download ${choice.selected.size} videos` `` |
| [src/views/download-playlist.ts:54](file:///C:/Users/A3/Claws/src/views/download-playlist.ts#L54) | `"Select None" : "Select All"` | `"Select none" : "Select all"` |
| [src/views/download-playlist.ts:59](file:///C:/Users/A3/Claws/src/views/download-playlist.ts#L59) | `"Select None" : "Select All"` | `"Select none" : "Select all"` |
| [src/views/download-playlist.ts:83](file:///C:/Users/A3/Claws/src/views/download-playlist.ts#L83) | `"Select None" : "Select All"` | `"Select none" : "Select all"` |
| [src/views/download-shared.ts:206](file:///C:/Users/A3/Claws/src/views/download-shared.ts#L206) | `"Estimate Sizes"` | `"Estimate sizes"` |
| [src/views/download-shared.ts:254](file:///C:/Users/A3/Claws/src/views/download-shared.ts#L254) | `"Stop Estimating" : "Estimate Sizes"` | `"Stop estimating" : "Estimate sizes"` |
| [src/views/queue.ts:266](file:///C:/Users/A3/Claws/src/views/queue.ts#L266) | `"Clear Finished"` | `"Clear finished"` |
| [src/views/queue.ts:273](file:///C:/Users/A3/Claws/src/views/queue.ts#L273) | `"Pause All"` | `"Pause all"` |
| [src/views/queue.ts:274](file:///C:/Users/A3/Claws/src/views/queue.ts#L274) | `"Resume All"` | `"Resume all"` |
| [src/views/queue.ts:275](file:///C:/Users/A3/Claws/src/views/queue.ts#L275) | `"Retry Failed"` | `"Retry failed"` |
| [src/views/queue.ts:289](file:///C:/Users/A3/Claws/src/views/queue.ts#L289) | `"Paste a Link"` | `"Paste a link"` |
| [src/views/library.ts:45](file:///C:/Users/A3/Claws/src/views/library.ts#L45) | `"…Use Show in Folder to find it."` | `"…Use Show in folder to find it."` |
| [src/views/library.ts:48](file:///C:/Users/A3/Claws/src/views/library.ts#L48) | `"Remove from Library?"` | `"Remove from library?"` |
| [src/views/library.ts:49](file:///C:/Users/A3/Claws/src/views/library.ts#L49) | `"Remove from Library"` | `"Remove from library"` |
| [src/views/library.ts:50](file:///C:/Users/A3/Claws/src/views/library.ts#L50) | `"Delete File Too"` | `"Delete file too"` |
| [src/views/library.ts:51](file:///C:/Users/A3/Claws/src/views/library.ts#L51) | `"Show in Folder"` | `"Show in folder"` |
| [src/views/library.ts:135](file:///C:/Users/A3/Claws/src/views/library.ts#L135) | `"Clear All"` | `"Clear all"` |
| [src/views/library.ts:141](file:///C:/Users/A3/Claws/src/views/library.ts#L141) | `"Clear Library"` | `"Clear library"` |
| [src/views/library.ts:161](file:///C:/Users/A3/Claws/src/views/library.ts#L161) | `"Check Again"` | `"Check again"` |
| [src/views/library.ts:249](file:///C:/Users/A3/Claws/src/views/library.ts#L249) | `"Paste a Link"` | `"Paste a link"` |
| [src/views/settings.ts:339](file:///C:/Users/A3/Claws/src/views/settings.ts#L339) | `"Check for Updates"` | `"Check for updates"` |
| [src/views/settings.ts:397](file:///C:/Users/A3/Claws/src/views/settings.ts#L397) | `sectionTitle("Quick Access")` | `sectionTitle("Quick access")` |
| [src/views/setup-panel.ts:70](file:///C:/Users/A3/Claws/src/views/setup-panel.ts#L70) | `"Setting Up…" : failed ? "Try Again" : "Set Up Everything"` | `"Setting up…" : failed ? "Try again" : "Set up everything"` |
| [src/views/setup-panel.ts:98](file:///C:/Users/A3/Claws/src/views/setup-panel.ts#L98) | `"Not Now"` | `"Not now"` |
| [src/views/shell.ts:123](file:///C:/Users/A3/Claws/src/views/shell.ts#L123) | `` `${open ? "Hide" : "Show"} Sidebar (Ctrl+Alt+S)` `` | `` `${open ? "Hide" : "Show"} sidebar (Ctrl+Alt+S)` `` |
| [src/views/spotify-match.ts:58](file:///C:/Users/A3/Claws/src/views/spotify-match.ts#L58) | `"Try Again"` | `"Try again"` |

**Paired test and dev-bench references affected by sentence casing:**
- [scripts/tests/spotify-match-dom.test.mjs:86, 88, 90, 105, 112](file:///C:/Users/A3/Claws/scripts/tests/spotify-match-dom.test.mjs#L86-L112): updates `buttons(..., 'Try Again')` to `'Try again'`.
- [scripts/dev/spotify-bench.mjs:122, 203, 208, 209, 212, 216, 217](file:///C:/Users/A3/Claws/scripts/dev/spotify-bench.mjs#L122-L217): checks `textContent === 'Audio Only'` and `__s.button('Try Again')` / `__s.click('Try Again')` (update alongside the view copy).

---

## 5. Detailed Findings — Decoration & Icons (`U7`, `U10`, `U11`, `U14`)

1. **[src/views/settings.ts:168](file:///C:/Users/A3/Claws/src/views/settings.ts#L168) — `U10`**
   - **Defect:** `fontStatusRow(true)` renders a decorative static green `<span class="tool-dot" data-state="ok">` next to `"Salted Flutters Polaris"`. The bundled font being loaded is not a live connection or service.
   - **Planned fix:** Render plain text `"Salted Flutters Polaris"` inside `<span class="row-note">` (or plain text node) without the green dot or `.font-status` wrapper.

2. **[src/views/settings.ts:361-370](file:///C:/Users/A3/Claws/src/views/settings.ts#L361-L370) — `U7, U10, U14`**
   - **Defect:** When `deno` is installed, the row title already reads `` `JavaScript runtime  Deno ${deno.version}` ``, while the right-hand control renders `<span class="font-status"><span class="tool-dot" data-state="ok"></span>Deno</span>` (or `jsRuntime`), repeating `"Deno"` and adding a static green dot that the sibling `yt-dlp` and `ffmpeg` rows directly above do not use.
   - **Planned fix:** Include `jsRuntime` in the row title when `deno` is absent (e.g., Node.js on `PATH`):
     ```ts
     `JavaScript runtime${deno ? `  Deno ${deno.version}` : jsRuntime ? `  ${jsRuntime}` : ""}`
     ```
     and pass `null` as the row control when `jsRuntime` is truthy, matching `ffmpeg` at [settings.ts:349-351](file:///C:/Users/A3/Claws/src/views/settings.ts#L349-L351).

3. **[src/styles/views.css:1036-1045](file:///C:/Users/A3/Claws/src/styles/views.css#L1036-L1045) — Guardrail cleanup**
   - **Defect:** `.font-status` and `.font-status .tool-dot` exist only for the two static dot rows above.
   - **Planned fix:** Delete lines `1036-1045` once [settings.ts:168, 365-370](file:///C:/Users/A3/Claws/src/views/settings.ts#L168) stop using `.font-status`.

4. **[src/views/download-batch.ts:96](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L96) — `U11`**
   - **Defect:** `"Retry"` inside `.entries-head` carries `icon: "retry"`, while all four sibling text buttons in the same header (`"Stop"`, `"Estimate sizes"`, `"Select all"`, `"Clear"`) have no icon.
   - **Planned fix:** Remove `icon: "retry"`.

5. **[src/views/setup-panel.ts:71](file:///C:/Users/A3/Claws/src/views/setup-panel.ts#L71) — `U11`**
   - **Defect:** Primary setup button `"Set up everything"` / `"Try again"` has `icon: failed && !working ? "retry" : "download"`, while sibling `"Not now"` has no icon and the label already states the action.
   - **Planned fix:** Remove `icon`.

6. **[src/views/settings.ts:340, 356, 375](file:///C:/Users/A3/Claws/src/views/settings.ts#L340-L375) — `U11`**
   - **Defect:** `"Check for updates"` (`icon: "retry"`) and both `"Install"` buttons (`icon: "download"`) carry icons, whereas Settings row buttons elsewhere (`"Choose…"`, `"Show"`, `"Font details…"`) are text-only.
   - **Planned fix:** Remove `icon` on all three tool buttons.

7. **[src/views/queue.ts:289](file:///C:/Users/A3/Claws/src/views/queue.ts#L289) & [src/views/library.ts:249](file:///C:/Users/A3/Claws/src/views/library.ts#L249) — `U11`**
   - **Defect:** Both empty states already display a 46px symbol above the title; adding `icon: "link"` inside the `"Paste a link"` primary button is decorative.
   - **Planned fix:** Remove `icon: "link"` on both buttons.

8. **[src/ui/icons.ts:20, 23, 26](file:///C:/Users/A3/Claws/src/ui/icons.ts#L20-L26) — `U11` / Unused code cleanup**
   - **Defect:** `open`, `clock`, and `sparkles` in `STROKE` are unused across `src/` (confirmed by symbol search).
   - **Planned fix:** Remove those three entries from `STROKE`.

---

## 6. Detailed Findings — Surface Rules (`--surface`: `U18` & `U25`)

All surface changes below preserve Claws's color palette, typography family/sizes, radii, glass materials, liquid edges, and layout structure, fixing only off-scale spacing (`U18` toward `4 / 6 / 8 / 12 / 16 / 20 / 28`, plus `2 / 6 / 10 / 14` inside controls) and misaligned edges (`U25`).

### A. Edge Alignment (`U25`)

1. **[src/styles/views.css:467-474](file:///C:/Users/A3/Claws/src/styles/views.css#L467-L474) — `.entries-head` (*Bars*)**
   - **Current:** `.entries-head` sets `align-items: center`, but its first child `.section-title` inherits `margin: 26px 6px 7px` from [views.css:45](file:///C:/Users/A3/Claws/src/styles/views.css#L45). Only `margin-right: auto` is overridden at `:473`, so the heading sits 19px higher (asymmetric vertical margin: 26px top vs 7px bottom) and pushes the count and action buttons 9.5px below the heading's center line.
   - **Proposed:** Follow the exact pattern already used by `.recent-head` ([views.css:641-649](file:///C:/Users/A3/Claws/src/styles/views.css#L641-L649)): move the outer vertical margin onto `.entries-head` (`margin: 26px 0 8px;`) and set `.entries-head .section-title { margin: 0 auto 0 6px; }`.

2. **[src/styles/base.css:506](file:///C:/Users/A3/Claws/src/styles/base.css#L506) & [src/styles/views.css:693](file:///C:/Users/A3/Claws/src/styles/views.css#L693) — `.large-title` & `.queue-summary` (*Text line*)**
   - **Current:** Text sitting directly on the window tint above glass panels uses three different left margins: `4px` on `.large-title` (`margin: 6px 4px 18px;`), `6px` on `.section-title` (`margin: 26px 6px 7px;`) and `.footnote` (`margin: 24px 6px 0;`), and `2px` on `.queue-summary` (`margin: 0 2px 10px;`).
   - **Proposed:** Align the minority to the `6px` window-tint text line:
     - `.large-title`: `margin: 6px 6px 16px;`
     - `.queue-summary`: `margin: 0 6px 10px;`

3. **[src/styles/views.css:22](file:///C:/Users/A3/Claws/src/styles/views.css#L22) — `.spotify-candidate` (*Text line*)**
   - **Current:** `.spotify-candidate` uses `padding: 12px 8px;` while `.spotify-identity` ([views.css:9](file:///C:/Users/A3/Claws/src/styles/views.css#L9)) and grouped `.row` items ([views.css:62](file:///C:/Users/A3/Claws/src/styles/views.css#L62)) align their content at `16px` horizontal inset inside `.group.glass`.
   - **Proposed:** `padding: 12px 16px;`

### B. Off-Scale Spacing (`U18`)

| Selector | File & Line | Current | Proposed | Reason |
| --- | --- | --- | --- | --- |
| `.nav-item` | [src/styles/base.css:344](file:///C:/Users/A3/Claws/src/styles/base.css#L344) | `gap: 9px;` | `gap: 8px;` | Off-scale odd gap → `8px` |
| `.tool-chip` | [src/styles/base.css:389](file:///C:/Users/A3/Claws/src/styles/base.css#L389) | `gap: 7px;` | `gap: 6px;` | Off-scale odd gap → `6px` |
| `.tool-line` | [src/styles/base.css:404](file:///C:/Users/A3/Claws/src/styles/base.css#L404) | `gap: 7px;` | `gap: 6px;` | Off-scale odd gap → `6px` |
| `.seg-item` | [src/styles/components.css:151](file:///C:/Users/A3/Claws/src/styles/components.css#L151) | `gap: 5px;` | `gap: 6px;` | Off-scale odd gap inside control → `6px` |
| `.bar` | [src/styles/components.css:292](file:///C:/Users/A3/Claws/src/styles/components.css#L292) | `margin-top: 7px;` | `margin-top: 8px;` | Off-scale odd margin → `8px` |
| `.section-title` | [src/styles/views.css:45](file:///C:/Users/A3/Claws/src/styles/views.css#L45) | `margin: 26px 6px 7px;` | `margin: 26px 6px 8px;` | Keeps documented `26px` top ([DESIGN.md:264](file:///C:/Users/A3/Claws/DESIGN.md#L264)); fixes `7px` bottom → `8px` |
| `.banner` | [src/styles/views.css:211](file:///C:/Users/A3/Claws/src/styles/views.css#L211) | `padding: 13px 16px;` | `padding: 12px 16px;` | Off-scale odd padding → `12px 16px` |
| `.notice` | [src/styles/views.css:294-295](file:///C:/Users/A3/Claws/src/styles/views.css#L294-L295) | `margin-top: 18px; padding: 13px 16px;` | `margin-top: 16px; padding: 12px 16px;` | Aligns with panel rhythm (`16px` / `12px`) |
| `.notice-text` | [src/styles/views.css:313](file:///C:/Users/A3/Claws/src/styles/views.css#L313) | `gap: 1px;` | `gap: 2px;` | `1px` → `2px` inside-control/text step |
| `.preview` | [src/styles/views.css:331](file:///C:/Users/A3/Claws/src/styles/views.css#L331) | `margin-top: 18px;` | `margin-top: 16px;` | `18px` → `16px` |
| `.actions` | [src/styles/views.css:427](file:///C:/Users/A3/Claws/src/styles/views.css#L427) | `margin-top: 18px;` | `margin-top: 16px;` | `18px` → `16px` |
| `.recent-head` | [src/styles/views.css:645](file:///C:/Users/A3/Claws/src/styles/views.css#L645) | `margin: 34px 0 7px;` | `margin: 32px 0 8px;` | `34px / 7px` → `32px / 8px` |
| `.job` | [src/styles/views.css:742](file:///C:/Users/A3/Claws/src/styles/views.css#L742) | `padding: 11px 12px 11px 14px;` | `padding: 10px 12px 10px 14px;` | `11px` → `10px` (matches `.row` vertical padding `10px`) |
| `.search` | [src/styles/views.css:820, 823](file:///C:/Users/A3/Claws/src/styles/views.css#L820-L823) | `gap: 7px; padding: 0 11px;` | `gap: 6px; padding: 0 10px;` | `7px / 11px` → `6px / 10px` (matches `.text-input` `10px` padding) |
| `.tile-text` | [src/styles/views.css:922](file:///C:/Users/A3/Claws/src/styles/views.css#L922) | `gap: 1px;` | `gap: 2px;` | `1px` → `2px` |

---

## 7. Kept on Purpose

- **Live badges (`U9`):**
  - Sidebar Queue count badge ([src/views/shell.ts:60, 195-200](file:///C:/Users/A3/Claws/src/views/shell.ts#L195-L200)): renders only when `store.activeCount() > 0` (`:empty { display: none }`).
  - Library duration badge ([src/views/library.ts:104](file:///C:/Users/A3/Claws/src/views/library.ts#L104)): renders only when `formatDuration(item.duration)` is non-empty.
- **Live tri-state tool status dots (`U10`):**
  - Sidebar tool chip dot ([src/views/shell.ts:75, 210](file:///C:/Users/A3/Claws/src/views/shell.ts#L210)): reflects live `ok` / `missing` / `error` status from `store.toolsKnown`, `store.toolError.ytdlp`, and `store.tools.ytdlp`.
  - First-run setup step dots ([src/views/setup-panel.ts:57](file:///C:/Users/A3/Claws/src/views/setup-panel.ts#L57)): reflects live per-tool `stepState(...)` (`ok` / `missing` / `error`).
- **Joined `·` metadata and telemetry lines (`U6`):**
  - Probed media summary ([src/views/download.ts:636](file:///C:/Users/A3/Claws/src/views/download.ts#L636), [src/views/download.ts:663](file:///C:/Users/A3/Claws/src/views/download.ts#L663), [src/views/download.ts:734](file:///C:/Users/A3/Claws/src/views/download.ts#L734), [src/views/download-shared.ts:292](file:///C:/Users/A3/Claws/src/views/download-shared.ts#L292), [src/views/spotify-match.ts:102, 119](file:///C:/Users/A3/Claws/src/views/spotify-match.ts#L102)).
  - Batch read counts and destination summary ([src/views/download-batch.ts:161, 184](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L161)).
  - Live transfer status, completion/kept-file note, and queue header summary ([src/views/queue.ts:65, 75, 313](file:///C:/Users/A3/Claws/src/views/queue.ts#L65)).
  - Library quality/size metadata and required Spotify provenance (`Source: YouTube match` at [src/views/queue.ts:139](file:///C:/Users/A3/Claws/src/views/queue.ts#L139) and [src/views/library.ts:95, 119](file:///C:/Users/A3/Claws/src/views/library.ts#L95), required by [docs/HANDOFF.md:38](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L38)).
- **Spotify track eyebrow label (`U1`):**
  - `"Spotify track"` at [src/views/spotify-match.ts:100](file:///C:/Users/A3/Claws/src/views/spotify-match.ts#L100): required by [docs/HANDOFF.md:38](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L38) to keep Spotify song metadata visibly separate from YouTube match candidates.
- **Sampled spring motion system (`U20`):**
  - [src/ui/motion.ts](file:///C:/Users/A3/Claws/src/ui/motion.ts) (`expandIn`, `collapseOut`, `settleIn`, `glideFrom`, `pop`, `completionFeedback`) and CSS spring curves are core specifications in [DESIGN.md:346-379](file:///C:/Users/A3/Claws/DESIGN.md#L346-L379), bounded (`≤ 12` rows / `≤ 40` tiles), reduced-motion-aware, and tested by `npm run test:motion`.
- **Unit-tested pure-logic helper strings (`U3`, `U4`):**
  - Technical constraints, formatting, and error diagnostics in [src/processing-options.ts](file:///C:/Users/A3/Claws/src/processing-options.ts), [src/resolution.ts](file:///C:/Users/A3/Claws/src/resolution.ts), [src/transcript-options.ts](file:///C:/Users/A3/Claws/src/transcript-options.ts), [src/download-estimates.ts](file:///C:/Users/A3/Claws/src/download-estimates.ts), [src/download-input.ts](file:///C:/Users/A3/Claws/src/download-input.ts), and [src/kept-files.ts](file:///C:/Users/A3/Claws/src/kept-files.ts) are directly asserted by `npm run test:logic` and match Rust backend invariants.

---

## 8. Owner Decisions / Tasks

- [x] Give the paste shortcut without the site count in [Download](../src/views/download.ts).
- [x] Remove the tile lift and artwork zoom from [Library styles](../src/styles/views.css) and [DESIGN.md](../DESIGN.md). Keep the actions that appear on hover and focus.
- [x] Associate the visible [Settings filename label](../src/views/settings.ts) with its input.

---

## 9. Step-by-Step Implementation & Verification Plan

### Phase 1 — Copy, Punctuation & Sentence Case (`U3–U5`, `U7`, `unslop 13, 14, 17, 28–30`)
1. Apply all 22 helper/empty-state/punctuation edits listed in Section 3 across:
   - [src/views/download.ts](file:///C:/Users/A3/Claws/src/views/download.ts)
   - [src/views/download-shared.ts](file:///C:/Users/A3/Claws/src/views/download-shared.ts)
   - [src/views/download-options.ts](file:///C:/Users/A3/Claws/src/views/download-options.ts)
   - [src/views/download-batch.ts](file:///C:/Users/A3/Claws/src/views/download-batch.ts)
   - [src/views/library.ts](file:///C:/Users/A3/Claws/src/views/library.ts)
   - [src/views/settings.ts](file:///C:/Users/A3/Claws/src/views/settings.ts)
2. Apply all 46 sentence-case edits (`unslop 17`) listed in Section 4 across those files plus [src/views/download-playlist.ts](file:///C:/Users/A3/Claws/src/views/download-playlist.ts), [src/views/queue.ts](file:///C:/Users/A3/Claws/src/views/queue.ts), [src/views/setup-panel.ts](file:///C:/Users/A3/Claws/src/views/setup-panel.ts), [src/views/shell.ts](file:///C:/Users/A3/Claws/src/views/shell.ts), and [src/views/spotify-match.ts](file:///C:/Users/A3/Claws/src/views/spotify-match.ts).

### Phase 2 — Decorative Dots & Icon Cleanup (`U7`, `U10`, `U11`, `U14`)
1. Remove the static green `.tool-dot` from `fontStatusRow` ([src/views/settings.ts:168](file:///C:/Users/A3/Claws/src/views/settings.ts#L168)) and collapse the duplicate runtime badge in the `JavaScript runtime` row ([src/views/settings.ts:361-370](file:///C:/Users/A3/Claws/src/views/settings.ts#L361-L370)).
2. Remove the orphaned `.font-status` and `.font-status .tool-dot` CSS rules ([src/styles/views.css:1036-1045](file:///C:/Users/A3/Claws/src/styles/views.css#L1036-L1045)).
3. Remove redundant icons on text buttons in [src/views/download-batch.ts:96](file:///C:/Users/A3/Claws/src/views/download-batch.ts#L96), [src/views/setup-panel.ts:71](file:///C:/Users/A3/Claws/src/views/setup-panel.ts#L71), [src/views/settings.ts:340, 356, 375](file:///C:/Users/A3/Claws/src/views/settings.ts#L340-L375), [src/views/queue.ts:289](file:///C:/Users/A3/Claws/src/views/queue.ts#L289), and [src/views/library.ts:249](file:///C:/Users/A3/Claws/src/views/library.ts#L249).
4. Delete unused `open`, `clock`, and `sparkles` entries in [src/ui/icons.ts:20, 23, 26](file:///C:/Users/A3/Claws/src/ui/icons.ts#L20-L26).

### Phase 3 — Surface Spacing & Edge Alignment (`U18`, `U25`)
1. Apply the four edge-alignment fixes (`U25`) and 17 off-scale spacing normalizations (`U18`) in:
   - [src/styles/base.css](file:///C:/Users/A3/Claws/src/styles/base.css)
   - [src/styles/components.css](file:///C:/Users/A3/Claws/src/styles/components.css)
   - [src/styles/views.css](file:///C:/Users/A3/Claws/src/styles/views.css)
2. Ensure all edits preserve `DESIGN.md` material alphas (`--gi`), reduced-transparency overrides, reduced-motion clamping, and layout structure.

### Phase 4 — Test Updates & Full Validation
1. Update the button-text assertions in [scripts/tests/spotify-match-dom.test.mjs:86, 88, 90, 105, 112](file:///C:/Users/A3/Claws/scripts/tests/spotify-match-dom.test.mjs#L86-L112) (`'Try Again'` → `'Try again'`) and the selector strings in [scripts/dev/spotify-bench.mjs:122, 203, 208, 209, 212, 216, 217](file:///C:/Users/A3/Claws/scripts/dev/spotify-bench.mjs#L122-L217) (`'Audio Only'` → `'Audio only'`, `'Try Again'` → `'Try again'`).
2. Run the project checks from [docs/HANDOFF.md#build-and-checks](file:///C:/Users/A3/Claws/docs/HANDOFF.md#L43-L60):
   ```powershell
   npm run test:fonts
   npm run test:spotify
   npm run test:logic
   npm run test:queue
   npm run test:motion
   npm run build
   git diff --check
   ```
3. Visually inspect the rendered app (`npm run dev` / headless Edge benches) at standard and narrow widths (`<= 1000px`), in both light and dark themes, with Transparency at `0`, `70`, and `100`, and with Backdrop set to `Solid` (`none`), confirming no holes, broken alignment, or clipped labels remain.
