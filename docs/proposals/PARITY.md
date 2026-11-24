# One Tend on Mac, iPhone/iPad and the web

Status: **proposal**, Phase 1 audit and Phase 2 recommendation (WAVE15, 2026-11-24).
Phase 3, the implementation, waits for the owner's approval.

The owner said: "iPhone (iOS, iPadOS) still feels different from the macOS Tend
app, and different from the web app. How do we give one consistent experience?"

The short answer: today they are three products, not one product with three
layouts. Each surface puts its own Home together and has its own strings. Each
also reads only part of the shared design tokens. So the same fleet shows
different counts, words, buttons and card order. The fix is not a visual pass.
It is a short list of written-down meanings that all three keep: one word for
each concept, one rule for each count, and one set of things each item lets you
do. Each client then lays those out in its own way. The screenshots narrow it further. Home's work cards
already look alike, so the drift is in the badges, the decision cards and the
words. The cheapest first step is to fix one workflow end to end (a question
in Your turn) on all three surfaces, not to build new machinery.

- Code inventory with file:line evidence for every claim below:
  [`parity/INVENTORY.md`](parity/INVENTORY.md).
- Screenshots: [`parity/shots/`](parity/shots/). They show the demo fixture
  (`docs/demo-setup.sh`) on one CI runner, rendered by all three surfaces
  (§1.5; the workflow is #707).

---

## Phase 1: the audit

### 1.1 Where each surface gets Home

| | Mac (Tend.app) | Web dashboard | iOS / iPadOS |
|---|---|---|---|
| Units (Work, agents) | Computed **in the app**. `FleetBoard` (`FleetHome.swift`) works on the IPC `fleet_snapshot`. | `/api/home` (tendd `home_board`). | `/api/device/home` (tendd `home_board`, the device projection). |
| Decisions (questions, results) | A separate poll in the app, `FleetDecisionFeed`. It reads the local `decision_queue`, each remote host's queue and the collaboration mailbox, and merges them itself. | Inside `/api/home`, already sorted into lanes by the server. `home-board.mjs` maps them to cards, then flattens, filters, folds and regroups them, and appends a separate history read. | Operate scope with the `home` feature: inside `/api/device/home`. Older operate hosts: `FleetLanes`, computed on the phone. Decide scope: no Fleet at all, only the decision queue plus history. |
| Groups, counts, folds | The app computes them. The lane count includes folded cards and follows the filters. The rail and the Dock have their own badges (the Dock adds unread). | The client regroups by project and **recounts** under its own filter and reviewed toggle, and adds history. Its fallback (`buildBoard`, one unit per agent) is used only when `/api/home` cannot be read. | Renders the server's groups. It folds and splits off "Can't be answered" itself. Finished adds all loaded history (recent history only on the fallback path). The tab badge counts only what this device can decide. |
| Proof that they agree | `home-board-parity*.json`: units only. Decisions, backlog, headlines and counts are "not compared yet". | Nothing. | Nothing beyond decoding fixtures. |

So one board is put together three ways. The lane assignment is shared by the
web and operate iOS, because both read tendd's `home_board`. The Mac makes its
own lane assignment, held to tendd's by a fixture for units only. **No two
surfaces count the same way.** The server's own `count` leaves out the
questions that can't be answered and counts every finished unit without
history (`home_board.rs:1800-1819`). Each client then adds or removes history,
filters and reviewed items in its own way. History is not even one source:
`/api/device/home` bundles history without closures, while iOS's separate
history read includes them.

### 1.2 Job × surface

**same** = same model, words and actions (layout may differ). **diff** =
present but different. **missing** = not there.

| Job | Mac | Web | iOS | Notes |
|---|---|---|---|---|
| Home lanes (Your turn / In progress / Finished, Backlog) | same | same | same | The lane names agree in EN and KO. Only the Mac has a "Queued" column. |
| Lane counts | diff | diff | diff | There are three counting rules (§1.1). The same fixture can show three different numbers. |
| Grouping by project, folds (empty / helpers / older / reviewed) | same | diff | diff | The web applies "reviewed" only on the server path. iOS adds "older decided". |
| Question: answer, options, Other… | same | same | same | |
| Question: close without answering | diff | diff | diff | Mac has **Dismiss / 닫기** and no Decline. The web has **Decline… / 거절…** in the ⋯ menu. iOS has **Decline / 거절** in the composer and no Dismiss. |
| Can't be answered: Keep, Close all | diff | same | same | On the Mac, Close all covers local rows only, and a remote row offers "Open on the Dashboard". The web and iOS work per host. |
| Review decision: Accept / Return… / Withdraw | same | same | same | Withdraw is in the context menu on the Mac and in ⋯ on the web and iOS. |
| Finished unit: Looks Good / Send Back, Review sheet | same | same | **missing** | iOS has neither in the app. Its push buttons "Looks good / Send back" actually mean Accept / Return on a review decision, so the same words mean different things. |
| Start work | diff | diff | diff | Mac: host, folder, team, plan, queue, plus a separate New Agent sheet. Web: Work id and task only. iOS: host, project, pipeline, Face ID. |
| Explore: hosts › sessions › windows › panes | same | same | **missing** | iOS goes hosts → agents. It parses `paneCount` and never shows it. |
| Agent detail | diff | diff | diff | Mac: an action bar (switch account, continue, move). Web: a unit inspector with reply and Open Pane. iOS: read-only. |
| Settings | diff | diff | diff | These are expected to differ (local app vs. dashboard vs. paired device). Only Notifications and the language should match in kind. |
| Empty state | same | same | diff | iOS shows the four-room mark at all-clear. The others show text. |
| Error: host offline / partial read | diff | diff | diff | Mac: an unreadable remote host shows **nothing**. Web: the raw English error and raw states (`auth_failed`). iOS: the server text verbatim, with no retry. |
| Notifications | diff | **missing** | diff | Mac: local notifications ("%@ asked you"). iOS: APNs ("Tend: %@ needs you"). The web has none. |

Of the 15 jobs, 3 are the same on all three surfaces (lane names, answering,
review decisions). 10 differ on at least one surface, and 3 are missing on
one (finished-unit review on iOS, Explore's pane tree on iOS, notifications on
the web). Notifications count in both groups, so the numbers add up to 16.

### 1.3 Vocabulary differences

These come from the three catalogs (Mac `Localizable.xcstrings` with 2369 keys,
iOS `Localizable.xcstrings` with 291 keys, web `home-strings.mjs` with 367 keys). Nothing is
shared. 135 keys overlap between Mac and iOS, as separate copies. The web copied the Mac
words by hand.

| Concept | Mac | Web | iOS |
|---|---|---|---|
| Close a live question | Dismiss / 닫기 | Decline… / 거절… | Decline / 거절 |
| Finished-result approval | Looks Good / 확인 완료 | Looks Good / 확인 완료 | Looks good / **좋습니다** (push only, = Accept) |
| Returned (history) | 반려함 | **돌려보냄** | 반려함 |
| Answered (history) | 답변 완료 | **답함** | 답변 완료 |
| Declined / withdrawn | 거절됨 / 철회됨 | **거절함 / 철회함** | 거절됨 / 철회됨 |
| Session ended | 세션이 종료되었습니다 | **세션 종료** | 세션이 종료되었습니다 |
| Pane | 페인, **패인** ("Pane gone"), **창** ("Choose in Pane") | Pane (Latin), 패인, 페인 | 페인 |
| Answer placeholder | 답변 | 답변 | **답** |
| Host state: version skew | Version skew / 버전 불일치 | raw `version_skew` | **Needs update / 업데이트 필요** |
| Host state: auth failed | Authentication failed / 인증 실패 | raw `auth_failed` | **Sign-in failed / 로그인 실패** |
| Inspector "now" label | Needs your decision / 결정이 필요합니다 | same | **NEEDS YOU / 내 차례** (upper-cased) |
| Decide elsewhere | Decide on the Dashboard (missing in KO) | Decide on that host / 그 호스트에서 결정하세요 | Decide this on %@… / %@에서 결정하세요 |
| Explore | Explore, Inbox, Ask, Shells (left in English) | EXPLORE, Panes \| Messages / **Pane \| 메시지** | **Hosts / 호스트** |

Untranslated strings also show up under KO. The Mac is missing 5 catalog keys. The web has its checks
headline, "an agent", its error messages, raw host states and two admin dialogs in English only.
Each surface resolves the server's `Text` keys (`waits_for_answer`, `stage_is`, …) with its own
code: `home-strings.mjs` `textOf()` on the web and `Board.swift` on iOS. The Mac has none, because it
never reads them.

### 1.4 Visual differences

One token source already exists: `apps/shared/Design/tokens.json` →
`tokens.css` + `TendDesignTokens.swift`. The surfaces adopt it unevenly:

- **Web** reads all of it.
- **Mac** reads its own `Mac.*` block (type, space, radius, inks). The lane hues are system
  `.orange/.blue/.green`, not the tokens. `cardYourTurn`, `stripeWidth`, `cardGap` and
  `inspectorWidth` go unused.
- **iOS** reads 15 colour tokens. Type, spacing and radii are hard-coded in
  `Theme.swift`. Secondary and tertiary inks are opaque hex values, not the token alphas.

| Token | tokens.json (= web) | Mac | iOS |
|---|---|---|---|
| Card / lane / control radius | 8 / 9 / 4 | 10 / 12 / 6 | 10 / 12 / 8 |
| Card padding / gap | 12 / 8 | 12 / 8 | 14 / 10 |
| Your-turn mark | 5 px orange stripe + tinted fill | 3 pt stripe, no fill | no stripe; orange 45 % border |
| Secondary / tertiary ink | rgba .50 / .26 | .55 / .45 | #5C5C5C / #6E6E6E |
| Card title | 14.5 | 14 semibold | `.body.semibold` (≈17, Dynamic Type) |
| Brand-mark ink | #16181D / #F2F3F5 | pure black / white | #16181D / #F2F3F5 |
| Live status mark | header | menu bar | **none** (all-clear and pairing only) |

Card anatomy is the most visible difference. On a question card, Mac and web
put the **title first and the meta (asker · host · time) last**. iOS puts
**time and asker on top and checks before the title**. A Your-turn card has a
stripe on two surfaces and a border on the third. (Full per-kind anatomy:
INVENTORY §3.)

Some differences are platform-appropriate and should stay: Dynamic Type and
44-pt touch targets on iOS, a 3-column board with an inspector on the Mac,
paged lanes on a phone. Those are layout choices. The table's mismatches are
not layout: they are the same role with different values.

### 1.5 Screenshots

All three surfaces render the demo fixture (`docs/demo-setup.sh`, one
`tend-demo` sandbox, one tendd with its dashboard) on the same GitHub
runner. The workflow is `.github/workflows/parity-shots.yml`, run on demand with the
`parity-shots` label. Files are named `<surface>-<job>-<variant>.png`.

Run [45349486538](https://github.com/Open330/tend/actions/runs/45349486538)
(PR #707) produced 316 images. The full set is in the run's `parity-shots`
artifact. A curated set of 22, light and EN unless noted, is in [`parity/shots/`](parity/shots/):

| Job | Mac | Web | iPhone / iPad |
|---|---|---|---|
| Home | ![](parity/shots/mac-home-light-en.png) | ![](parity/shots/web-home-desktop-light.png) | ![](parity/shots/ios-home-yourTurn-2-iphone.png) ![](parity/shots/ios-home-ipad.png) |
| Home, KO | ![](parity/shots/mac-home-light-ko.png) | ![](parity/shots/web-home-desktop-light-ko.png) | |
| Review decision | (on the Review sheet) | ![](parity/shots/web-review-card-desktop-light.png) | ![](parity/shots/ios-review-card-card-iphone.png) |
| Question | | ![](parity/shots/web-question-desktop-light.png) | ![](parity/shots/ios-question-card-iphone.png) |
| Start work | ![](parity/shots/mac-home-start-work-light-en.png) | ![](parity/shots/web-start-work-desktop-light.png) | ![](parity/shots/ios-start-work-iphone.png) |
| Explore / Hosts | ![](parity/shots/mac-explore-light-en.png) | ![](parity/shots/web-explore-desktop-light.png) | ![](parity/shots/ios-hosts-iphone.png) |
| Agent detail | ![](parity/shots/mac-detail-agent-light-en.png) | ![](parity/shots/web-agent-detail-desktop-light.png) | ![](parity/shots/ios-agent-detail-iphone.png) |
| Not connected | ![](parity/shots/mac-home-unconnected-light-en.png) | | ![](parity/shots/ios-error-unreachable-iphone.png) |

**What the screenshots add to the code audit**

- **Home's unit cards already match.** All three show the same lane names,
  project groups, card order inside a card (title, stage bar, agent, inset
  headline, then status, logo, host, project and time) and status words. The
  differences are the Your-turn mark and fill (a stripe on Mac, a stripe and
  tinted fill on the web, an orange border on iOS) and the type scale. Home is
  not where the three products feel different.
- **The badges disagree for one fleet.** Your turn is 9 on the web and iOS
  (8 on the Mac, which was captured before tendd added a recovery question).
  But the Mac rail badge says **6**, the web's says **9** and the iPhone tab
  says **3**. This is §1.1's count problem, seen on screen.
- **Decision cards are where they part.** The same review decision appears
  twice below. The web puts the kind on top, wraps the content in an inset box,
  puts the actions inside that box, and puts the host and time at the bottom.
  iOS puts the kind, project and time on top, has no inset box, puts the
  actions in tinted capsules and puts ⋯ at the right. The question card on iOS
  offers only "Answer…"; Decline is inside the composer.
- **Chrome:** the web shows the four-room mark (top left) and iOS shows none.
  The Mac's off-screen capture shows "Connecting to tendd… 0 hosts" in its
  status bar while Home is full. That is probably an artifact of off-screen
  rendering, but it is worth checking in P2.
- **Not captured:** there is no Backlog lane anywhere, because the fixture has
  none. The empty Home is not shown, because emptying it would mean deciding
  things. There is no iOS review detail, because iOS has none. iPad landscape
  images come out rotated 90° (`simctl` behaviour). The portrait iPad
  shot above is correct.

---

## Phase 2: the proposal

The goal is **one meaning, three layouts.** For every item, all three surfaces should use the same word,
apply the same count rule and offer the same thing to do. Each platform keeps
its own placement, disclosure and native affordances: Changes and Finder on
the Mac, Face ID and Dynamic Type on iOS, the keyboard on the web.

The first draft centred on one server projection that would carry every
count and an ordered action list, plus a generated string catalog and a CI
gate on each UI. Codex's challenge (§2.8) showed that this was more than the
problem needs. Most of the visible drift is words, card order and a
few missing actions. Fixing those is cheap, and none of it needs new
infrastructure. The five candidates, re-evaluated:

### 2.1 Shared projection: **yes, for domain facts and capabilities; no, for ordered action lists**

tendd already assigns the lanes for the web and for operate iOS. Extend
`home_board` with the facts that clients now work out differently:

- **Named counts**, each with one definition: lane items, actionable
  decisions, unanswered questions, questions that can't be answered, and coverage (complete,
  partial or truncated). The definitions also state how history and reviewed
  items count toward a total. Local filters apply to the shared fold
  membership. They do not create new rules.
- **Per item:** kind, fold reason, and the viewer's capabilities (may answer,
  decline, keep, accept, return, withdraw, or mark as looked at) with a refusal
  reason when it may not. The server's enforcement stays the authority. Where
  and how a surface shows an action is up to the client. Close all is a host
  action over the listed questions, not a per-card action.
- **One history source and window** for every client.
- **Decide scope keeps its current queue.** It gets the same presentation
  rules, not a second feed. A test checks that a decide-scope device never
  sees pane ids, sockets, agent counts or inferred work. iOS's operate projection
  removes those on purpose (`devices/operate.rs:831-867`).

Fallbacks stay until a capability and consumer matrix shows that nothing needs
them. The Windows proxy, older hosts and refused optional reads are covered by
JS tests today.

### 2.2 One vocabulary: **a small curated glossary now; generation only if drift recurs**

The two Swift catalogs share 135 keys. Only two of them differ in wording,
plus one difference in the shape of a positional format. That is not a reason
to merge the catalogs. The proposal instead:

- writes a **glossary** in `apps/shared/Strings/GLOSSARY.md`: one EN and KO term
  per concept, with a semantic key;
- adds a **check** (one JS test) that reads both xcstrings files and
  `home-strings.mjs` for the glossary keys. It compares the wording and the
  shape of each parameter or format. It does not compare the raw English
  source text.

Each platform keeps its own string format. That means plural objects and the
`l10n-sync.py` review state on Mac and iOS, function-valued templates on the
web, and named loc-keys for APNs. Terms to settle in the glossary:

- Pane is **페인**.
- Returned is 반려함, Answered is 답변 완료, Declined / Withdrawn are 거절됨 / 철회됨.
- A live question is closed with one verb, **Decline / 거절**. The Mac already
  sends `declined`, with a fixed message. The web and iOS send the person's
  text. The owner decides whether the reason is optional everywhere. This is
  different from dismissing a report and from closing a question that can't be answered.
- Host states use the Mac's words.
- Push buttons say **Accept / Return**, not "Looks good / Send back", because
  that is the decision they make.

### 2.3 Design tokens: **adopt the attention role first; values only, native theme code stays**

Through the existing generator and native theme mappings:

1. Give **Your turn's attention role** (hue, mark, fill) one token, used
   by all three surfaces. Today the mark is a stripe on two surfaces and a border on the third.
2. Set **contrast targets and the title/meta hierarchy** before any
   numeric radius. iOS's stronger secondary inks were chosen for measured
   contrast (`Theme.swift:71-108`). They become the target, not the outlier.
3. Use **one radius set** after that.

iOS keeps Dynamic Type and its touch targets. The Mac keeps keyboard
navigation and its compact and inspector variants. A live status mark on iOS is
optional and comes last.

### 2.4 Card anatomy and actions per kind: **yes, done workflow by workflow**

There is one spec table per item kind: the order of its parts and its
capability set, in the glossary's words. It is applied one workflow at a time,
in the current renderers. The first workflow is a **question in Your turn**,
covering answer, options, Decline, Keep, Close all, a refusal, partial
coverage, and title-first order on iOS. The next is a **review decision**.
After that comes a **finished unit**.

iOS has no finished-unit "Looks Good" today. The write that records it now is
operator-only and tied to a revision (`home.rs:157-189`,
`server.rs:5643-5706`), and the device projection leaves out the `review_key`.
If the owner wants it on the phone, it needs a narrow device write with a
stale-revision check and an audit line. It must never go through the
pending-review endpoint. Send Back and the full Review sheet stay
Mac and web for now. Send Back types into a pane, which is a different
authority and failure model.

### 2.5 Parity check: **independent fixtures in the existing suites, plus one smoke capture**

A check that compares every UI with tendd's current output would pass a
regression that all of them share. Instead:

- **Expected outcomes are written independently**: for an operator, a
  decide-only device, an observe-only remote host, truncated and unknown
  entries, reviewed units and a stale action. Each is a fixture.
- **The existing suites assert against them.** Rust has `home_board` parity, the Mac has
  `HomeBoardParityTests`, the web has `dashboard-home-page.test.mjs`, and iOS has `BoardTests`.
  Extend the fixtures from units to decisions, counts and capabilities.
- **Screenshots of the visible hierarchy** come from `parity-shots.yml`
  (this audit), run on demand or on a label, for human review. They are not a gate.

### 2.6 Different on purpose

These stay different:

- **Layout:** the Mac's 3 columns plus inspector, the iPad split view, the
  iPhone's paged lanes, the responsive web.
- **Platform affordances:** shortcuts, Face ID, Changes and Finder on the Mac.
- **Settings:** each surface configures different things.
- **Notifications:** local on the Mac, APNs on iOS. Web notifications are a
  separate decision.
- **The Mac's data path:** see the next section.

The Mac Home publishes only visible changes and pauses its polls when hidden.
It also keeps slow remote hosts from holding up local decisions
(`FleetHomeBoard.swift:5-74`). Moving it onto IPC `home_board` is **not
proposed**. That would make sense only after recurring, measured drift, and
only behind an opt-in shadow comparison. The comparison would have to cover
dashboard-off, an old daemon, a partial Fleet, local actions, freshness and
latency. The dashboard's build also holds a shared-cache lock today and can
build twice for reviewed marks (`home.rs:79-106`). The Mac shares the
*meanings* through the fixtures instead.

### 2.7 Phased plan

| Phase | What | Size |
|---|---|---|
| P1 | **Glossary + check** (2.2). Fix the KO mismatches in §1.3, the Mac's 5 missing keys, the web's untranslated strings and the push button words. | S |
| P2 | **The question in Your turn, end to end** on all three surfaces (2.4). Decline everywhere, iOS card order title-first, one attention mark (2.3 step 1), and refusal and partial-coverage text. | M |
| P3 | **Count and capability semantics** (2.1). Named fields in `home_board`, one history source, independent fixtures in the Rust, Swift and JS suites (2.5). | M |
| P4 | **Web and operate iOS read those facts.** Remove the client recounts and keep the fallbacks. Then the review-decision and finished-unit workflows. | M |
| P5 | **Tokens:** contrast and hierarchy targets, then the radius set (2.3 steps 2–3). | S–M |
| P6 | *Only if prioritized:* finished-unit marking on iOS through a narrow device route. | S–M |
| — | *Deferred until drift is measured:* catalog generation, the Mac on IPC `home_board`. | — |

P1 and P3 can start together. P2 depends on P1's words. P4 depends on P3.

**Recommendation:** approve P1–P5. Start with P1 and P2. Together they remove
most of what the owner sees as "feels different": the words, the card order,
the Your turn mark and the close button. They need no new infrastructure.
P3 and P4 then make the counts agree.

### 2.8 Codex challenge (2026-11-24)

Codex reviewed the first draft against the code. 50 focused Node tests
passed, and it read the Swift catalogs and the native paths. Its points, and
what changed in this draft:

1. **§1.1 overstated the derivations.** The web maps lanes the server already
   assigned, its fallback is one unit per agent, and decide-scope iOS has no
   Fleet. **Accepted.** §1.1 is corrected.
2. **"One server count for every badge" is wrong.** Lane items, actionable decisions,
   questions that can't be answered, unread and coverage are different numbers.
   History is two sources. **Accepted.** The draft now uses named counts and one history source.
3. **Dismiss and Decline are one concept.** The Mac already sends `declined`. Align the
   verb, and decide whether the reason is optional. Keep it distinct from dismissing a report.
   **Accepted.**
4. **Relabel the iOS push buttons now.** Treat finished-unit marking on a phone as a
   separate, narrow, revision-bound write. Defer Send Back and Review. **Accepted** (P1, P6).
5. **The projection should carry facts and capabilities, not ordered action arrays.** Keep
   native affordances and the decide-scope privacy, and keep the fallbacks. **Accepted.**
6. **A generated catalog merge is not needed** for two differing values. Use a curated
   glossary and a check, and keep each platform's formats. **Accepted.**
7. **Tokens:** iOS's inks are deliberate. Lead with the attention role and contrast, not
   radii. **Accepted.**
8. **A UI-versus-tendd semantic gate passes a regression they all share.** Use
   independent fixtures in the existing suites and a smoke capture. **Accepted.**
9. **Mac over IPC with a one-release retirement** is unsafe without a shadow
   comparison. **Accepted.** It is deferred.

One point stays where it was. The attention mark and the title-first card order
are part of the first step (P2), not something done later. They are the most
visible differences in the screenshots, and they cost little.
