---
name: Claws
description: A Windows desktop video downloader built as clear Apple-style glass, with thin tint materials over the real blurred desktop and controls that move on springs.
colors:
  blue: "#0a84ff"
  blue-fill: "#0071e3"
  accent-text-light: "#0062c8"
  accent-text-dark: "#64b5ff"
  danger-fill: "#d92d20"
  red: "#ff453a"
  red-text-light: "#c4170f"
  red-text-dark: "#ff7a72"
  orange: "#ff9f0a"
  orange-text-light: "#9a5700"
  orange-text-dark: "#ffb340"
  green: "#30d158"
  green-text-light: "#0b7a31"
  green-text-dark: "#4bdd78"
  window-tint-light: "#f2f2f6"
  window-tint-dark: "#18181c"
  material-light: "#ffffff"
  chrome-light: "#f8f8fb"
  material-thick-dark: "#2c2c32"
  material-regular-dark: "#2e2e34"
  chrome-dark: "#24242a"
  solid-base-light: "#f2f2f7"
  solid-base-dark: "#1c1c1e"
  ink-light: "rgba(0, 0, 0, 0.86)"
  ink-secondary-light: "rgba(60, 60, 67, 0.78)"
  ink-dark: "rgba(255, 255, 255, 0.94)"
  ink-secondary-dark: "rgba(235, 235, 245, 0.68)"
  fill-light: "rgba(120, 120, 128, 0.14)"
  fill-dark: "rgba(120, 120, 128, 0.24)"
  separator-light: "rgba(60, 60, 67, 0.14)"
  separator-dark: "rgba(255, 255, 255, 0.1)"
  light-close: "#ff5f57"
  light-minimize: "#febc2e"
  light-zoom: "#28c840"
typography:
  large-title:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "28px"
    fontWeight: 700
    lineHeight: 1.12
    letterSpacing: "-0.016em"
  headline:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    lineHeight: 1.24
    letterSpacing: "-0.014em"
  title:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 650
    lineHeight: 1.3
    letterSpacing: "-0.01em"
  section-title:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 650
    letterSpacing: "-0.005em"
  body:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.42
    letterSpacing: "-0.003em"
  label:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 500
  caption:
    fontFamily: "'Claws Polaris', -apple-system, BlinkMacSystemFont, 'Segoe UI Variable Display', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "11.5px"
    fontWeight: 400
    lineHeight: 1.38
  mono:
    fontFamily: "'SF Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "12px"
    fontWeight: 400
rounded:
  xs: "6px"
  sm: "8px"
  md: "12px"
  sidebar: "14px"
  lg: "16px"
  sheet: "20px"
  xl: "22px"
  capsule: "999px"
spacing:
  xxs: "4px"
  xs: "6px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  xxl: "28px"
components:
  button-primary:
    backgroundColor: "{colors.blue-fill}"
    textColor: "#ffffff"
    rounded: "{rounded.sm}"
    height: "28px"
    padding: "0 13px"
  button-prominent:
    backgroundColor: "{colors.blue-fill}"
    textColor: "#ffffff"
    rounded: "{rounded.capsule}"
    height: "34px"
    padding: "0 20px"
  button-secondary:
    backgroundColor: "{colors.fill-light}"
    textColor: "{colors.ink-light}"
    rounded: "{rounded.sm}"
    height: "28px"
    padding: "0 13px"
  button-plain:
    textColor: "{colors.ink-secondary-light}"
    rounded: "{rounded.sm}"
    height: "28px"
    padding: "0 13px"
  button-danger:
    backgroundColor: "{colors.danger-fill}"
    textColor: "#ffffff"
    rounded: "{rounded.sm}"
    height: "28px"
  button-small:
    rounded: "7px"
    height: "24px"
    padding: "0 10px"
  button-glass:
    backgroundColor: "{colors.fill-light}"
    rounded: "14px"
    height: "28px"
    width: "30px"
  chip:
    backgroundColor: "{colors.fill-light}"
    rounded: "13px"
    height: "26px"
    padding: "0 12px"
  chip-selected:
    backgroundColor: "{colors.blue-fill}"
    textColor: "#ffffff"
  segmented-item:
    rounded: "7px"
    height: "24px"
    padding: "0 13px"
  switch:
    backgroundColor: "{colors.blue-fill}"
    rounded: "12px"
    height: "24px"
    width: "40px"
  text-input:
    backgroundColor: "{colors.fill-light}"
    rounded: "7px"
    height: "28px"
    padding: "0 10px"
  search-field:
    backgroundColor: "{colors.fill-light}"
    rounded: "14px"
    height: "28px"
    width: "210px"
  url-bar:
    rounded: "26px"
    height: "52px"
    padding: "0 8px 0 18px"
  glass-panel:
    rounded: "{rounded.lg}"
  sidebar-panel:
    rounded: "{rounded.sidebar}"
    padding: "0 10px 10px"
  nav-item:
    textColor: "{colors.ink-light}"
    rounded: "{rounded.sm}"
    height: "32px"
    padding: "0 10px"
  row:
    height: "46px"
    padding: "10px 14px 10px 16px"
  toast:
    rounded: "{rounded.xl}"
    padding: "9px 10px 9px 14px"
  sheet:
    rounded: "{rounded.sheet}"
    width: "300px"
---

# Design System: Claws

## Overview

**Creative North Star: "Clear Glass on the Desktop"**

Claws is a pane of clear glass set on the user's own desktop. Nothing is painted behind the interface. Windows blurs whatever is behind the window (accent-policy blur-behind, without Acrylic's tint or grain), and the UI lays thin tint materials over that real blur in Apple's order: a window tint across the whole pane, a floating sidebar, grouped content panels, and chrome for the bars and overlays that sit above scrolling content. The colour on screen comes from the user's wallpaper and windows, not from the app. The brief is to be as close to Apple's interface as possible with a strong focus on blur, so the system follows current macOS conventions directly: traffic-light controls that stay put, an inset floating sidebar, a large title that collapses into the toolbar, inset-grouped rows, a sliding segmented control, alert sheets and capsule toasts.

It is a working tool. Density is compact and calm: 13px body, 28px controls, 46px rows, a 900px content column. Saturated colour is limited to one Apple blue and the system status hues. Motion is physical: transitions run on sampled springs, controls compress under the pointer and spring back, and selection travels as one shape.

Salted Flutters Polaris leads the type stack and is bundled with Claws in four weights. System fonts cover glyphs outside the bundled subset. Settings reports whether Polaris is available and links to its project page.

**Key Characteristics:**
- The window is the glass: the OS blurs the desktop, the UI only tints. In-app blur is reserved for chrome that sits over in-app content.
- One Transparency value (`--gi`, 0 to 1, default 0.7) scales every material alpha; blur radii do not depend on it.
- Panels carry a 1px liquid edge, a masked gradient ring brightest at top-left and bottom-right.
- Light and dark themes share every token name; only values change.
- Hairlines (0.5px) and inset rims separate things; no stroked borders.
- Every transition that moves something uses a spring variable (`--spring-smooth`, `--spring-snappy`, `--spring-bouncy`).

## Colors

Neutral grey-white and graphite tints over whatever the desktop shows, with one Apple blue and Apple's system status hues.

### Primary
- **Apple Blue** (blue): progress fills, text selection highlight (at 32%), and the focus ring hue in dark mode.
- **Deep Action Blue** (blue-fill): every fill that carries white text: primary and prominent buttons, selected chips, switch on-state, slider progress, checked check circles. Also the sidebar selection (at 16% light) and the focus ring in light mode (at 55%). White on it is 4.7:1.
- **Accent Text** (accent-text-light / accent-text-dark): blue used as text or icon on a material: sidebar icons, toast actions.

### Neutral
- **Window Tint** (window-tint-light / window-tint-dark): the base pane over the desktop blur, at alpha `0.8 - gi*0.48` light and `0.82 - gi*0.42` dark.
- **Thick Material** (material-light / material-thick-dark): the floating sidebar, alpha `0.8 - gi*0.3` light, `0.68 - gi*0.24` dark, always denser than Regular.
- **Regular Material** (material-light / material-regular-dark): every glass panel, alpha `0.72 - gi*0.26` light, `0.62 - gi*0.24` dark.
- **Chrome Material** (chrome-light / chrome-dark): toolbar once scrolled, toasts and sheets, alpha `0.9 - gi*0.22` light, `0.86 - gi*0.2` dark. The only material that also blurs.
- **Solid Base** (solid-base-light / solid-base-dark): the opaque window when the backdrop is set to None; panels and sidebar then become colour-mixes of it.
- **Ink** (ink / ink-secondary): primary and secondary text on materials. A third role, **Label** (`rgba(0,0,0,0.8)` light, `rgba(255,255,255,0.88)` dark), is for text that sits directly on the window tint: large title, section titles, empty-state titles, footnotes.
- **Fills** (fill at 14/24%, fill-2 at 22/34%, fill-3 at 30/44% of `rgb(120,120,128)`): resting, hover and strong tints for secondary controls, chips, inputs, tracks and hover rows.
- **Rims** (white at 95%/35% light, 22%/6% dark): the liquid edge and 0.5px inset highlights.
- **Separator** (separator-light / separator-dark): 0.5px hairlines between rows.

### Status
- **Red, Orange, Green** (red, orange, green): status dots and progress fills for failed, paused and completed. As text they always use their `-text` variant, darkened in light and lightened in dark. Error notices use red at 14% as a wash.
- **Traffic lights** (light-close, light-minimize, light-zoom): window controls only; grey (fill-3) when the window is inactive.

### Named Rules
**The Borrowed Colour Rule.** The app paints no colour field and no background image. Colour on screen comes from the desktop through the glass; materials stay neutral.
**The White-Text Floor Rule.** White text sits on blue-fill or danger-fill, never on the lighter Apple Blue. Coloured text on a material uses the `-text` variants.

## Typography

**Display and Body Font:** Bundled Salted Flutters Polaris, referenced internally as `Claws Polaris` to avoid installed-font substitution (with Segoe UI Variable Display/Text, Segoe UI, system-ui for missing glyphs)
**Mono Font:** SF Mono (with ui-monospace, Cascadia Mono, Consolas)

**Character:** One platform face throughout, set the way Apple sets it: heavy, tightly tracked titles over a small, slightly negative-tracked 13px body. Rank comes from size and weight, never from case.

### Hierarchy
- **Large title** (700, 28px, 1.12, -0.016em): the page title at the top of each scroll view, in Label colour.
- **Headline** (700, 20px, 1.24, -0.014em): the link preview title, clamped to three lines and balanced. Empty-state titles use 19px / 700 / -0.012em.
- **Title** (650, 14px, -0.01em): the toolbar title that appears once the large title scrolls away; the alert sheet title is 14px / 700.
- **Section title** (650, 13px, -0.005em): group headings above inset-grouped panels, in Label colour.
- **Body** (400, 13px, 1.42, -0.003em): rows, entries, nav (500), list titles (500-600). The URL field is 15px.
- **Label** (500, 12-13px): buttons (13px; small 12px), chips and segments (12.5px), sub-lines and status lines (12px, secondary ink).
- **Caption** (400, 11.5px, 1.38): row hints (max 54ch), tile meta, tool status. Badges and duration overlays are 11px / 600.
- **Mono** (12px; inline code 11px on a fill chip): file paths, templates and tool versions.

### Named Rules
**The Interface Font Rule.** Use bundled Salted Flutters Polaris for interface text, with system fallbacks for missing glyphs. Keep its four web font weights and license in the project.
**The Tabular Rule.** Any number that updates or aligns (progress, speeds, counts, durations, dates, versions, stepper values) gets `font-variant-numeric: tabular-nums`.

## Layout

A two-column window grid: the sidebar column (248px; 220px under 1000px wide) and the content column. The sidebar is a floating pane inset 8px from the window's top, bottom and left edges with a 14px radius, so its corners sit concentric with the window. Collapsing it (toolbar toggle or Ctrl+Alt+S) springs the grid column to 0 while the pane slides 28px left and fades; the traffic lights and the toggle are fixed at 20px/22px from the window corner and never move. Ctrl+1 to Ctrl+4 switch sections.

The toolbar (52px) floats over the content with no fill at the top of a page; once the large title has scrolled under it, it gains chrome material, a bottom hairline, and the page title fades up into it. The scroll area pads 52px top, 28px sides (20px under 1000px) and 40px bottom, with a stable scrollbar gutter, and centres a 900px page.

Content is inset-grouped: a glass panel holds rows at least 46px tall (10px 14px 10px 16px padding), label left and control right (control at most 62% wide), separated by hairlines that start at the text inset (16px), or past the thumbnail in media lists. Section titles sit 26px above their group. The library is an auto-fill grid of tiles at a 208px minimum with 16px gaps. Spacing runs on a 4/6/8/12/16/20/28 rhythm. Under 1000px rows wrap and the preview stacks.

## Elevation & Depth

Depth is material thickness first, shadow second. The desktop blur happens once, in the OS; panels add tint, a liquid edge and a soft two-layer drop. Only surfaces that sit over in-app content blur again: the scrolled toolbar and toasts (`blur(30px) saturate(180%)`), the alert sheet (also `--chrome-blur`), the sheet's dimming scrim (6px), and the library duration badge (14px over the thumbnail). Shadows are soft, low-contrast and cool-black; there are no hard offsets.

### Shadow Vocabulary
- **Panel** (`box-shadow: 0 1px 2px rgba(0,0,20,0.06), 0 8px 24px rgba(0,0,20,0.07)`): every glass panel at rest.
- **Panel lifted** (`box-shadow: 0 2px 4px rgba(0,0,20,0.08), 0 14px 34px rgba(0,0,20,0.14)`): library tile on hover, with a 2px rise.
- **Sidebar** (`box-shadow: 0 6px 22px rgba(0,0,20,0.1)` plus 0.5px rim insets): the floating pane.
- **Float** (`box-shadow: 0 12px 36px rgba(0,0,20,0.14), 0 2px 6px rgba(0,0,20,0.08)`; dark `0 16px 44px rgba(0,0,0,0.42), 0 2px 8px rgba(0,0,0,0.26)`): toasts.
- **Sheet** (`box-shadow: 0 26px 70px rgba(0,0,12,0.36), 0 4px 12px rgba(0,0,12,0.18)`): the alert sheet.
- **Control lift** (`box-shadow: inset 0 0.5px 0 rgba(255,255,255,0.35), 0 1px 2px rgba(0,40,120,0.3)`): primary buttons; danger uses a red-tinted twin.
- **Knob** (`box-shadow: 0 2px 4px rgba(0,0,0,0.24), 0 0 0 0.5px rgba(0,0,0,0.04)`): switch knob, slider thumb and segmented thumb (lighter).
- **Focus** (3px ring at the focus colour; 3.5px on the URL bar): `:focus-visible` outline, or a box-shadow ring on fields.

### Named Rules
**The One Blur Rule.** Content panels and the sidebar never use backdrop-filter; behind them is only the window tint, and the desktop is already blurred. Blur in-app only where real in-app content passes underneath.
**The Transparency Rule.** Material alphas are always derived from `--gi`. Transparency at 0 and the None backdrop drive it to 0; the reduced-transparency media query also makes the window, panels and sidebar opaque, exactly like Solid. Those overrides sit after the rules they replace, because a media query adds no specificity.
**The Flat Ancestor Rule.** Never animate `opacity` or `filter` on an ancestor of a blurring element; it flattens the glass. The blurring element itself may fade (toolbar chrome, toasts, sheet).

## Shapes

Continuous, generously rounded forms on a 6/8/12/16/22px ladder: 6px for small thumbnails, 7px for small buttons, text fields and segment items, 8px for buttons and nav items, 12px for notices and the preview image, 14px for the sidebar pane, 16px for panels and tiles, 20px for the sheet, 22px for the preview panel and toasts. Prominent actions, chips, the search field, the URL bar, switches and the window-chrome toggle are capsules (radius half the height). Traffic lights, status dots and check circles are circles. Edges are drawn by the liquid edge ring, 0.5px inset rims and hairlines, never a stroked border.

## Components

### Glass panel (signature)
The base surface. Regular material, 16px radius, the panel shadow, and the liquid edge: a `::before` ring 1px wide, a 160-degree gradient from bright rim through soft rim to clear and back, masked to the border box so it follows any radius. No blur. Used for the URL bar, banners, preview, grouped rows, the queue list, library tiles, the library search field and the fatal screen. Under the None backdrop it becomes a near-opaque mix of the solid base.

### Buttons
- **Shape:** 28px tall, 8px radius, 13px / 500; small is 24px, 7px radius, 12px; icon buttons are square.
- **Primary:** blue-fill, white text, control-lift shadow.
- **Prominent:** the primary in capsule form for the page's main action: 36px with 18px radius in the URL bar, 34px with 17px radius for Download, 13.5px text.
- **Secondary:** fill with a 0.5px inset top rim; hover deepens to fill-2. **Plain:** transparent with secondary ink, fill on hover. **Danger:** danger-fill with white text, used only in confirmation sheets. **Glass:** the 30x28 capsule sidebar toggle in window chrome, fill with full rims.
- **Press:** compresses to 0.95 in 0.1s ease-out, then springs back on `--spring-bouncy` when released. Disabled is 40% opacity and inert.

### Segmented control
A fill track, 9px radius, 2px inset, with a white (graphite in dark) thumb at 7px radius that slides on `--spring-snappy`, positioned by `--i` over `--n` items. Items 24px, 12.5px / 500; unselected items use secondary ink.

### Chips
26px capsules (13px radius), fill, 12.5px / 500; hover fill-2; selected is blue-fill with white text. Press compresses to 0.94 and springs back.

### Switch, slider, stepper
- **Switch:** 40x24 capsule, fill-3 off and blue-fill on, with a 20px white knob that travels on `--spring-snappy`. While held, the knob stretches to 25px wide toward the travel direction, as on iOS.
- **Slider:** 4px track, blue-fill progress, 20px white thumb that scales to 1.15 while dragged.
- **Stepper:** a fill capsule (9px radius) holding two small plain buttons and a 600-weight tabular value.

### Inputs / Fields
- **Text field:** 28px, 7px radius, fill, no border; focus adds a 3px focus ring as box-shadow.
- **Search:** a 210px glass capsule (28px, 14px radius) that springs to 260px wide and takes the ring on `:focus-within`.
- **URL bar:** a 52px glass capsule with a link icon, a 15px borderless field and the prominent button; the focus ring is added to the panel shadow on `:focus-within`. The native outline is suppressed only on inputs whose container draws this ring.

### Navigation
Sidebar items are 32px, 8px radius, 13px / 500, with a 17px icon in accent text. Hover shows fill on non-current items. The current page is marked by one travelling indicator (selection tint, 8px radius) that moves between items on `--spring-snappy` rather than each item lighting up. The Queue item carries a fill-2 capsule badge (18px, 11px / 600, tabular) that pops on a bouncy spring when the count rises. The footer holds the tool-status chip: a 7px status dot (green ok, orange missing, red error) with an 11.5px tabular line, plus a progress bar while setting up.

### Toolbar and large title
The large title belongs to the page; the toolbar title belongs to the window. As the large title passes under the 52px toolbar, the toolbar gains chrome material and the 14px title rises 4px into place on `--spring-smooth`. When the sidebar is closed the toolbar pads 138px left to clear the fixed window controls.

### Window controls
12px red, yellow and green circles with 8px glyphs revealed on group hover or keyboard focus, a 0.5px dark inset edge, and a brightness press; grey when the window is inactive.

### Progress and activity
4px rounded bar on fill-2; the blue fill scales by `--p` (transform `scaleX`) and glides forward on `--spring-smooth`, turning green, orange or red with job status. Jumps snap instead of gliding (see Motion). Indeterminate fill is a 34% segment crossing the track by transform at 1.4s. The activity indicator is Apple's eight-spoke spinner, fading each spoke over 0.8s.

Download rows only show a number they can stand behind:
- **Known size.** A percentage weighted by bytes across the video and audio streams.
- **Estimated size.** "About 42%", with "about 3:10 left".
- **Unknown size.** The indeterminate fill with "1.2 GB downloaded" and the speed.
- **Processing.** The indeterminate fill, with no time estimate.
- **Saved.** The bar reaches 100% only once the file is in the output folder.

### Toasts and alert sheet
- **Toast:** bottom-centre chrome capsule (22px radius, float shadow, chrome blur) that springs up from 26px below at 0.9 scale on `--spring-bouncy`; leaves in 0.22s. Green or red status icon, 500-weight text, an optional 26px capsule action in accent text.
- **Sheet:** a 300px chrome alert, 20px radius, centred title and message, full-width stacked 30px buttons, springing in from 0.88 scale over a 20% dim scrim with a 6px blur.

### Rows, entries, tiles
Grouped rows follow Layout. Playlist entries and recent items use 64x36 thumbnails at 6px; queue jobs use 88px 16:9 thumbnails at 7px with 600-weight titles and 12px tabular status lines coloured by status. Entry checks are 20px circles that compress while pressed and fill blue-fill when checked. Library tiles are glass panels with a 16:9 artwork that scales to 1.03 on hover, a blurred dark duration badge at top-right, and a gradient action bar of small translucent white buttons revealed on hover or focus. Small additions of queue rows expand in from zero height and removed rows collapse out in place; new stage content settles in from 0.985 scale and 6px below (see Motion).

### Empty states
Apple's content-unavailable layout: one 46px symbol in secondary ink, a 19px / 700 title, a line of 13px secondary help (max 44ch), and optionally one action. No tile, card or illustration behind the symbol.

## Motion

Motion says what just happened, a little faster than reading would. It never stands in for real state: a status, a percentage, a check or a file only appears when the backend reports it, and an animation never delays it, adds to it or fakes it.

**Springs.** `src/ui/motion.ts` samples three springs into CSS `linear()` easings, published as `--spring-<name>` and `--spring-<name>-d`: smooth (critically damped, about 660ms) for layout and progress, snappy (about 440ms, a hint of overshoot) for small controls and selection, bouncy (about 700ms) for things that arrive and press release. Scripted effects call `spring(name)` and animate with `Element.animate`.

**Effects in use.**

| Effect | Where | Movement | Rules |
| --- | --- | --- | --- |
| Lookup settles into its details | Download stage | Opacity 0 → 1, 6px rise, 0.985 → 1 scale, smooth spring | A newer lookup cancels the older settle. Focus stays in the link field. Errors appear at once. |
| Type selection | Video / Audio / Transcript segmented control | The one thumb slides by `translateX` on snappy | No second selection system. Arrow keys move selection and focus together. |
| Page selection | Sidebar | One indicator travels by `translateY` on snappy; the page swap cross-fades in 0.2s | — |
| New queue rows | Queue | Height 0 → measured, opacity and 0.98 scale, smooth spring | Only additions of 12 rows or fewer. Rows already there when the Queue opens never animate, and neither do any of a larger addition's rows, built 40 per frame. All the new rows of a frame are measured in one read. |
| Removed queue rows | Queue | Collapse to zero height in place, up to 320ms; the rows below follow | A row still arriving has its arrival cancelled. More than 12 rows leaving at once (Clear Finished) go without movement. A leaving row is never moved in the list. |
| Row order | Queue | None | Jobs keep their creation order: nothing in the queue reorders, so there is no reorder animation. Rows are only moved if somehow out of order, without movement. |
| Progress | Queue rows and the sidebar's tool-setup bar | The fill glides forward on smooth (transform only, no layout reads) | These bars snap, with the transition off for one frame, on a value lower than the one shown (a retry, the next tool's download), on leaving the indeterminate sheen and while the window is hidden; the tool bar also snaps when it moves on to the next tool. The Download view's setup banner and the bars in Settings are redrawn on every progress event, so they show each new value at once without gliding. A queue row also snaps on its first paint, any status change (completion, pause, failure, retry, resume) and a size becoming known or unknown. The fill never passes 100% and never slides backwards. Unknown sizes, processing and transcripts keep the indeterminate sheen. |
| Completion | Queue status line | The static check and "Saved to …" appear in the same frame as the event; the line then rises 3px from 45% opacity in 220ms | Only on the change to completed, never per progress event. Leaving completed (retry, removal) cancels it. |
| File drag pickup | Drag handles in the Queue and Library | Blue outline and grabbing cursor | Shown exactly as long as the native drag runs; the outcome comes from the native operation. |
| Badge | Queue item in the sidebar | Scale 0.6 → 1 on bouncy | When the active count rises. |
| Controls | Buttons, chips, switch, slider | Press compresses quickly; release springs back | — |

**Cancellation.** `expandIn`, `collapseOut` and the completion movement track the one movement each element runs, so a newer one, a retry, a removal or leaving the view (`cancelMotion`) cancels it instead of stacking another. Leaving the Queue cancels the rows' movements and takes rows that are still collapsing out of the page at once, so nothing keeps running on detached rows.

**Focus and repeated presses.** A row's buttons are replaced when its status changes; keyboard focus moves to the same action, or else to the first one that cannot cancel or remove. After a press, the buttons that replace the pressed one ignore activation for 600ms (each press's 1.5s fallback, for a command that changes nothing, clears only that press's focus target and guard, never a newer press's) and a held Enter does not repeat, so a double click or a second Enter cannot resume what was just paused or retry what was just cancelled. Removing the row that holds focus moves focus to the neighbouring row's first button that cannot cancel or remove (or to the empty state's action), guarded the same way. A row collapsing out is `inert` and hidden from assistive technology.

**Reduced motion and hidden windows.** With Reduce motion on, every CSS transition and animation clamps to 0.01ms and scripted effects do not start: rows appear and go at once, the bar jumps, the thumb and indicator move instantly. Static indicators stay: the check, the status colours, the indeterminate fill's colour and the drag outline. While the window is hidden, no scripted effect starts (rows, completion, lookup settle, badge pop, sheet departure, page transition) and progress snaps.

**Cost.** Movement uses transform and opacity wherever practical. Only row arrival and removal animate height, capped at 12 rows and measured in one read per frame. Nothing reads layout per progress event; progress repaints only its own row. `scripts/dev/motion-bench.mjs` measures each effect, and `scripts/dev/queue-bench.mjs` measures 500 rows with 8 active transfers. Both run in headless Edge against the dev mock; results are in `docs/performance/queue-ui.md`.

No decorative or looping animation in the working downloader; the indeterminate sheen and the spinner loop only while work of unknown length is actually running.

## Do's and Don'ts

### Do:
- **Do** build content surfaces from the glass panel: regular material, liquid edge, panel shadow, no blur.
- **Do** derive every material alpha from `--gi`; check new surfaces at Transparency 0 and 100, in both themes, and with the None backdrop.
- **Do** reserve backdrop-filter for chrome over in-app content: `blur(var(--chrome-blur)) saturate(180%)`.
- **Do** run every moving transition on a spring variable: `--spring-smooth` for layout, `--spring-snappy` for small controls and selection, `--spring-bouncy` for things that arrive and for press release.
- **Do** compress pressed controls quickly (0.1s ease-out, scale 0.94-0.95) and let them spring back on release.
- **Do** use inset-grouped rows with hairlines that start at the text inset for settings and lists.
- **Do** put white text only on blue-fill or danger-fill, and coloured text through the `-text` variants.
- **Do** honour reduced motion (transitions and animations clamp to 0.01ms; scripted effects do not start) and keep the static indicator that motion would have drawn attention to.
- **Do** return an animation's handle, so a retry, removal or newer change can cancel it.

### Don't:
- **Don't** paint a background behind the UI: no colour orbs, gradients or images; the desktop is the backdrop.
- **Don't** add backdrop-filter to panels or the sidebar; the desktop is already blurred behind them.
- **Don't** animate opacity or filter on an ancestor of a blurring element.
- **Don't** use fixed cubic-bezier curves for movement where a spring variable fits.
- **Don't** let motion stand in for state: no fabricated progress, no delayed completion, no optimistic drag success, no bar sliding backwards.
- **Don't** animate hundreds of rows, read layout per progress event, or add decorative looping animation.
- **Don't** put a tile, card or illustration behind an empty-state symbol.
- **Don't** add hard offset shadows, stroked borders or flat saturated panel fills.
- **Use** the bundled Polaris web fonts for interface text and include their license.
