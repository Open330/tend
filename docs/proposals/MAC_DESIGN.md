# Tend for Mac: a modern, clean, professional look

Wave 9 (owner, 2026-11-19): "Improve the app screens to be more modern, clean
and professional." Earlier complaints in the same vein: raw fonts, low
readability. This is a visual pass. It keeps the information architecture
(Home, Explore, Settings; three lanes; the activity bar's places; the editor
tab strip), the app icon, every affordance and every behaviour. Where
something moves, the information moves with it.

- **Before:** every screen drawn off-screen against the demo fixture fleet by
  `DesignCaptureTests` (`scripts/mac-shots/capture.sh`, the **Mac screens**
  workflow on a PR labelled `mac-shots`), light and dark, English and Korean.
  Nothing was launched on anyone's screen.
- **After:** the proposed system in SwiftUI, `docs/proposals/mac-design/mockups.swift`,
  drawn with `ImageRenderer` to `docs/proposals/mac-design/after-*.png`:
  - Home at 1440×900 and at the window's minimum, 920×580;
  - Home with nothing to show;
  - Explore;
  - the menu bar popover;
  - each in light and dark, English and Korean.

  The content is the fixture fleet's, hand-placed. The mockups are static:
  ImageRenderer draws neither materials nor scroll views, so the sidebar
  shows its opaque fallback and an overfull lane is clipped with a fade.
  Proposal code only; it never ships.

## 1. Critique

What the screens and the code say today, against current macOS practice
(Finder, Mail, Xcode, System Settings on macOS 26):

1. **A type ramp exists, but it is bypassed.** `TendType` names three roles
   (body, detail, meta). Beside them sit 228 `.font(.system(size:))`
   literals in 20 sizes (7 to 60pt, including 13.5 and 14.5), plus 88
   semibold and 57 medium overrides. A card title, a lane title and a page
   title differ by half a point here and there. That reads as "raw fonts":
   a card has no clear first, second and third thing.
2. **No spacing rhythm.** `.padding` takes 20 different values (1 to 32).
   Neighbouring views disagree by 1 to 2 points, which reads as untidy even
   when the eye cannot say why.
3. **Container inside container.** A Your turn card is a lane panel, then a
   bordered card, then an orange-tinted border and fill, then an inset box,
   then one bordered box per option: up to five nested outlines around one
   decision. Each level adds a stroke and padding and takes width from the
   text.
4. **Colour used for emphasis more than meaning.** Orange is a lane title, a
   card border, a card fill, an icon and a word. Blue is the selected
   activity-bar rule, the status-bar pill and the primary button. When
   everything is tinted, the one card that needs the person stands out less.
   Several state words also fail contrast: today's orange and green tokens
   are 2.3:1 and 2.2:1 on white.
5. **Workbench chrome borrowed from an IDE.** A 46pt flat activity bar with a
   blue selection rule, a tab strip with a coloured top rule and a solid blue
   `tendd` pill in the status bar are VS Code's vocabulary. On macOS they read
   as a web app in a window: no sidebar material, a hard edge between the
   strip and the content, and a status bar louder than the content.
6. **Toolbar without hierarchy.** The toolbar holds the layout picker,
   Review, Filter, Hosts, automation and Start Work, in five different
   styles.
7. **Repeated hints.** Each lane repeats its hint ("Decide or review",
   "Agents are on it", "Ready for the next instruction"), and the hint is cut
   in narrow lanes ("Ready for the next in…").
8. **What is already right:** Settings follows the system grouped form and
   needs only spacing and type alignment. The identity discs, product marks,
   the stage bar's states and the four-room brand are distinctive and stay.

(An earlier draft called "(1794812553)" in a title a request-id leak. It is
not: it is fixture text that the scratch daemon appends on purpose,
`apps/tend-ios/Scripts/scratch-tendd.sh`, and Home shows titles verbatim.)

## 2. The system (Tend for Mac first)

### Type ramp
Eight roles, three weights (regular, medium, semibold), all the system font
(SF Pro, with Apple SD Gothic Neo for Korean):

| Role | Size / leading | Weight | Use |
|---|---|---|---|
| Page | 20 / 24 | semibold | the content's title (Home, an agent's name) |
| Card title | 14 / 19 | semibold | what a card or row is about |
| Section | 13 / 16 | semibold | lane headers, sheet sections |
| Body | 13 / 18 | regular | **decision text**: questions, options, answers, notes |
| Secondary | 12 / 16 | regular | the line under a title, evidence labels |
| Label | 11 / 14 | medium | short labels only: kind, status word, stage, badge |
| Meta | 11 / 14 | regular, tabular digits | who, where, when |
| Mono | 12 / 16 | regular | commands, paths, pane text |

Decision-bearing prose (the question, its context, options, the asker's
recommendation and rationale, answers) is Body, 13pt. Evidence keeps its
full accessible label and its reveal path. 11pt is for short metadata and
labels only.

**Text size.** macOS has no user-facing Dynamic Type for SwiftUI apps
(`dynamicTypeSize` does not follow a system setting on the Mac), so this
proposal does not claim system text scaling. The roles are fixed sizes. If
the owner wants larger text, an app-level scale is a separate item: it needs
a prototype at larger sizes, minimum row heights instead of fixed ones, and
wrapping metadata. Controls keep their native sizes.

### Spacing
A 4-point scale: 2, 4, 8, 12, 16, 20, 24, 32. Card padding 12, gap between
cards 8, between lanes 16 (8 at the minimum width), content margin 24 (12 at
the minimum), toolbar 52, tab strip 36, status bar 24, tree rows 26 (a
minimum, not a locked height).

### Surfaces, radii, elevation
Four surfaces:

| Surface | Light | Dark | How it separates |
|---|---|---|---|
| Sidebar / tab strip | `#EFEFF2` | `#232326` | opaque fallback of the system sidebar material |
| Content | `#FFFFFF` | `#1C1C1E` | the window |
| Lane | `#F4F4F6` | `#232325` | a well: fill only, no border |
| Card | `#FFFFFF` | `#2C2C2F` | fill and an 8% hairline; in light also a 1pt, 6% shadow |

The activity bar and sidebar list draw the system sidebar material
(`NSVisualEffectView`, `.sidebar`, behind-window, following the window's
active state); the tab strip and status bar take its opaque value. Under
Reduce Transparency or Increase Contrast the column is that opaque value too,
the one every ink is checked against. The shared `side-bar` and
`activity-bar` tokens stay the web's and iOS's. Content and cards stay opaque. Radii: control 6, option group 8, card 10,
lane 12, all continuous corners.

### Colour roles and contrast

Text contrast per surface (sRGB, composited):

| Light | sidebar | content / card | lane | selected sidebar row |
|---|---|---|---|---|
| label (86% black) | 14.0 | 15.5 | 14.5 | 12.1 |
| secondary (55%) | 4.57 | 4.74 | 4.64 | 4.39 → label ink on selected rows |
| state ink: orange `#A85400` / green `#16743A` / blue `#0066D6` / red `#C8302E` / purple `#8E3FC0` | 4.65 / 5.09 / 4.73 / 4.67 / 5.05 | 5.34 / 5.84 / 5.42 / 5.36 / 5.80 | 4.86 / 5.32 / 4.94 / 4.88 / 5.28 | under 4.5 → neutral word |
| white on accent `#0066D6` | | 5.42 | | |

| Dark | sidebar | content | lane | card | selected sidebar row |
|---|---|---|---|---|---|
| label (88% white) | 12.4 | 13.4 | 12.5 | 11.2 | 9.0 |
| secondary (58%) | 6.2 | 6.5 | 6.2 | 5.7 | 5.0 |
| state hue: orange / green / blue / red / purple | 7.7 / 7.9 / 5.5 / 5.1 / 5.8 | 8.3 / 8.6 / 6.0 / 5.6 / 6.3 | 7.7 / 7.9 / 5.5 / 5.1 / 5.8 | 6.8 / 7.0 / 4.9 / 4.6 / 5.2 | blue 3.9, red 3.7 → neutral word |
| white on accent `#0A6CDA` | | 5.04 | | | |

Rules that follow from the tables:
- **Text that informs** is label or secondary, never tertiary. Meta
  (time, host, counts) is secondary. Tertiary (45% / 42%, about 3.3:1) is
  for icons, separators and placeholders only.
- **A state word** uses the state ink in light and the hue in dark. On a
  selected row the word is neutral (label ink) and the dot keeps the colour.
  Every text on a selected row is label ink.
- **The accent fill** is `#0066D6` in light and `#0A6CDA` in dark. White on
  it is 5.4:1 and 5.0:1. Today's system blue `#007AFF` carries white at only
  4.0:1. The accent appears on the one primary button, badges, links and
  selection focus; nothing else is blue unless it is the working state.
- **State hues** (dots, the waiting bar, stage segments, lane dots) keep
  today's tokens and meanings: orange waits for you, blue working, green
  done, red error, purple limited. A colour is never the only cue: each has
  its word, and the stage bar's active segment also differs in shape.
- **Stage tracks** that are still to come are 16% / 20% ink (1.6:1 against
  the card). They are a shape cue; the n/N count and the stage name carry the
  information in text.
- **Pane output** keeps the terminal palette and Ghostty's minimum-contrast
  rule (`TerminalCapturePalette`). These text rules do not apply inside it.
- To verify per component in the build PRs: hover, pressed and keyboard focus
  (the system focus ring); Increase Contrast (stronger hairlines and
  secondary ink); Reduce Transparency (the opaque sidebar shown here); 1× and
  2× displays.

### Components
- **Card:** one container. The kind icon, kind label and time make the first
  line. Then the title (Card title), the detail (Secondary), and **what tend
  checked first, then the asker's recommendation in its own words**, as
  today. Then the options, the actions, and the who-and-where line (product
  mark, host disc, host, project, helper count) on every card in every lane.
  A card waiting for the person has a 3pt orange bar at its leading edge and
  says so in its orange kind label. It has no orange border or fill.
- **Stage bar:** today's semantics: one 4pt segment per stage, coloured by
  its state (done, working, your turn, waiting), the active segment notched.
  The current stage's name and the done/total count sit under it.
- **Options:** one grouped list with hairline dividers and numbered chips,
  not one box per option. Long options wrap.
- **Buttons:** one filled accent primary per view (Start Work). Secondary is a
  soft fill without an outline (Review, Filter, Hosts, Accept). Tertiary is
  accent text (Return…, Answer…). Numeric badges stay where the number means
  something.
- **Toolbar:** Page title, then the layout picker, Review with its count,
  Filter, Hosts with its health and the host popover, the automation entry, a
  separator, and Start Work. At the minimum width, Review and Filter drop
  their words; their icons and help stay.
- **Lane:** a well with a header (state dot, Section title, count). The hint
  sentence leaves the populated board: it becomes the lane's empty-state
  text (see the empty mockup) and the header's help and accessibility hint.
- **Activity bar:** the same places and numeric badges, on the sidebar
  surface, 64pt wide with the traffic lights at the top. Selection is a soft
  rounded pill, not a blue rule.
- **Tab strip:** still the title bar. The selected tab is the content colour,
  joined to the content with rounded top corners. Window-inactive and
  keyboard-focus states remain distinct.
- **Status bar:** Meta text, with tendd's state as a dot and a word. When
  tendd is connecting, unhealthy or out of usage, it keeps its explicit text
  and colour; only the healthy state turns quiet.
- **Explore:** the Needs attention section stays above the topology, with
  the topology controls (sort, filter, new) in the header. Rows are 26pt,
  name first with layout priority, then status as a dot and a word, and
  counts in secondary Meta.
- **Menu bar popover:** the current grouping (agent / host), drill-in, reply
  and the Inbox, usage, Open Tend and Settings entries, on cleaner rows (an
  identity tile, the name, one line, a status tag). The mockup shows no
  keyboard shortcut that the app does not have.

### Shared tokens
`apps/shared/Design/tokens.json` generates the web's CSS and the Swift
tokens that iOS also reads. The first build PR adds **Mac-scoped** roles
(type, spacing, the four surfaces, state ink, the accent fill) beside the
shared keys and changes no shared value. Shared identity and state meanings
stay as they are. Changing a value for the web or iOS goes through their
owners (%77, %56), with their own captures.

### Acceptance details for the component PRs (agreed with the reviewer)
- Numbered option chips use label ink (secondary on the chip fill is 4.4:1
  in dark).
- The primary button's pressed state is an opaque, darker fill. Today's
  `tint.opacity(0.78)` would put white on 3.7:1. Real `Button` semantics,
  focus, disabled and destructive states, and accessible names on
  icon-only controls stay.
- StatusTag takes its context. On any selected or tinted surface the word is
  label ink.
- The stage bar keeps today's projection. The marker sits only on
  `FleetWork.currentStageIndex`, since stages can run in parallel. It needs a
  non-colour cue that survives 1×: light done and your-turn fills are about
  2.5:1 on white and the track is 1.5:1. VoiceOver reads the stage name,
  state and count together. A single-stage Finished card has no bar, as today.
- Cards wrap the existing contents and interactions; the chrome changes, the
  behaviour does not. Full titles stay accessible when visually limited.
- At narrow widths the who-and-where row may wrap or split. The Hosts control
  keeps its label, help and connected / degraded cue.

## 3. Density

Measured on the same fixture fleet: the before set from the CI capture, the
after set from the mockups with the same cards, groups and titles.

| | Before 1440×900 | After 1440×900 | After 920×580 (minimum) |
|---|---|---|---|
| First-viewport cards: Your turn / In progress / Finished | 4 / 4 / 5 | 4 / 4 / 5 | 2 / 2 / 3 |
| Titles cut with "…" | 2 of 13 visible | none (wrap to 3 lines) | some in Korean at the minimum |
| A card's state said twice ("Working" in a box and in the status line) | every running and finished card | never | never |
| Status, product, host, project, time on every card | yes | yes, one row | yes |

The after sample places only the first 4 / 4 / 5 cards, so it shows that a
viewport reads the same first cards, not its maximum capacity. At normal width
it shows as many cards per viewport as today. It drops the
inset box that repeated the state, and the lane hint, so titles get room to
wrap instead of being cut. The capture has no minimum-width before shot, and
neither set yet covers more than one host or the compact layout after. The
first screen PR measures both layouts on the real views at both widths, with
several hosts and long English and Korean text.

## 4. Before and after

`docs/proposals/mac-design/before-*.png` are taken from the capture.
`after-*.png` are the mockups: Home (normal, minimum width and empty), the
decision cards, Explore and the menu bar popover, in light and dark, English
and Korean. The PR (#645) puts them side by side. The full before set (76
screens, including all of Settings, Start Work, Ask, Inbox, onboarding and a
window before tendd answers) is the **Mac screens** artifact.

The capture does not yet include the Review sheet: pressing Review through
the accessibility tree did not open it off-screen. Mockups for the Review
and Start Work sheets come with the screen PRs.

## 5. Plan

Small PRs, each with before and after captures from the workflow. None mixes
in behaviour changes. None may regress the app's redraw work: the `app`
session owns the AppModel and FleetHome hot paths and is asked first.

1. **Tokens (Mac-scoped):** the eight type roles, spacing scale, surfaces,
   state ink and accent fill; `TendType` grows to the eight roles.
2. **Components:** Card, OptionList, StageBar (restyled, same semantics),
   button kinds, StatusTag, LaneHeader, status bar, activity bar and tab
   strip on the tokens. Prototype the sidebar material, with Reduce
   Transparency's opaque fallback.
3. **Screens:** Home (both layouts, lanes, toolbar, empty states), Explore
   rows, the Review and Start Work sheets, the menu bar popover, then
   Settings spacing and onboarding.
4. **Sweep:** replace the remaining size literals with roles, guarded by a
   check that fails on a new `.font(.system(size:))` outside the ramp.
