# Tend client parity inventory: Mac, web, iOS

Read-only audit of `/Users/june/personal/tend-wt-wave15` (HEAD `e54ad2d5`, v0.10.37). The cross-surface sections (0–4) come first. Appendices A–C hold the per-surface detail with file:line citations.

Path abbreviations:
- Mac: `apps/tend-macos/Sources/…`
- Web: `crates/tend/src/dashboard/web/…`. `H` is home.mjs, `HB` home-board.mjs, `HS` home-strings.mjs.
- iOS: `apps/tend-ios/Sources/…`

KO values:
- Mac and iOS: the `ko` stringUnit of each app's own `Resources/Localizable.xcstrings`.
- Web: the `KO` table in `HS`.

## 0. Per-job matrix (summary; details and citations in the appendices)

| Job | Mac | Web | iOS |
|---|---|---|---|
| **Home: data** | Computes its own board. `FleetBoard` (`FleetHome.swift:151-644`) runs over IPC `fleet_snapshot` (`AppModel.swift:1009-1080`). Decisions come from a separate `FleetDecisionFeed` poll: local `decision_queue`/`review_list`, remote hosts' queues, and the collaboration mailbox. Its parity with tendd is tested only for units (fixtures README: "Not compared yet: decisions, the backlog, headlines…"). | Reads `/api/home` (server `home_board`) through `fromServerBoard` (`HB:594`). It then flattens and re-groups by project, **recomputes counts** (`HB:643`, ignoring the server `count`), applies folds, rebuilds decision cards from the raw `entry`, and appends `/api/decisions/history` unsorted. If that read fails, it falls back to a full client port, `buildBoard` (`HB:410`), over `/api/decisions` and `/api/fleet`. It never calls `/api/device/*`. | Two paths. With operate scope and the `home` feature, it renders `/api/device/home` and only folds and splits the Can't be answered group (`Board.swift:422-497`). Otherwise Your turn comes from `/api/decisions`, and In progress / Finished are computed locally by `FleetLanes` (`Fleet.swift:195-228`). Decided items always come from `/api/decisions/history`. |
| **Lanes EN / KO** | Backlog\*, Queued\*, Your turn, In progress, Finished / 백로그, 대기열, 내 차례, 진행 중, 완료 | Backlog\*, Your turn, In progress, Finished / same KO. No Queued column. | Backlog\*, Your turn, In progress, Finished / same KO. No Queued. |
| **Lane count** | `works + decisions`. Includes folded cards; excludes reviewed-hidden cards and Can't be answered (`FleetHomeView.swift:747`). Rail badge counts units only, not decisions. | `visible − reviewed`, client-side. Rail badge and `(N)` tab title = Your-turn count. | `FoldedLane.count` = items − reviewed. Finished adds **all** history (`past.count`). Tab badge = decidable pending only. |
| **Folds** | empty / helpers / older (24h) / reviewed. Your turn never folds. | same four. "reviewed" only on the server path. | same four, plus "%lld older decided" / "이전에 결정한 것 %lld개" for history. |
| **Phone layout** | n/a | ≤760px: one lane at a time behind segments | <900pt: count strip plus paged TabView |
| **Question actions** | option rows → "Answer…"/"Other…" → **"Dismiss"/"닫기"**. Composer: Cancel, Send. **No Decline.** | option rows → "Answer…"/"Other…". Composer: Cancel · Send. ⋯ menu → **"Decline…"/"거절…"**. | option rows → "Answer…"/"Other…". Composer: Cancel · **Decline/거절** · Send. **No Dismiss/close** on a live question. |
| **Can't be answered** | at the end of Your turn, collapsed. Rows: "Keep"/"유지" (local) or "Open on the Dashboard" (remote). "Close All…" works on local rows only. | same group. Keep per host; "Close all"/"모두 닫기" works per host, remote hosts included. Inline confirm. | same group. "Close All…" or "Close All on %@…" per host. Confirmation dialog. |
| **Review decision** | "Accept"/"수락" · "Return…"/"돌려보내기…". Withdraw only in the context menu. Another host's item → "Decide on the Dashboard" (**missing from the KO catalog**). | Accept · Return… · ⋯ Withdraw | Accept · Return… · ⋯ Withdraw |
| **Finished-unit review** | Inspector ghost "Looks Good"/"확인 완료" (`review_record`). Review sheet ⇧⌘R: Later · Send Back · Open · **Looks Good**. Modes Cards/Inbox/Digest. Digest has "Summarize" (Ask). | Same sheet: Later · Send Back · Open · Looks Good, with the same keys and modes. No Summarize. `POST /api/home/reviewed`. | **None.** There is no Looks Good, no Review sheet, and no Send Back on cards. "Looks good"/"좋습니다" and "Send back"/"돌려보내기" exist only as **push actions**, and those map to a review *decision* `accepted`/`returned`. |
| **Start work** | Sheet: task, Where (host, folder "Choose…"), Team (pipeline/template), Plan (dry run), More options (Work ID, workspace, external, context, plan-only). Buttons: Cancel, Add to Queue, Start Work / Build Plan. There is also a separate New Agent sheet. | Dialog: "Work" id (required, regex) and "What to do". Cancel · Start. No host, folder, or template. `POST /api/work-control/up`. | Sheet: task, Where (Host, Project from work-options), Team (Pipeline). Toolbar Cancel / Start, with Face ID. `POST /api/device/work`. |
| **Explore** | Rail: Home, Explore, Inbox, Ask, Shells, Files. Host › session › window › pane tree, status groups, sort options. | Explore with a Panes \| Messages switch. Host tree / status groups. Pane output over SSE, Send / Interrupt. Remote panes have no output. | Hosts list → host → agents only. **No session or pane tree.** `paneCount` is parsed but never shown. |
| **Agent detail** | Status dot, title, status word, session id, action bar (switch account, send "continue", move account, Live Watch). Tabs Summary / Conversation / Shell. | Unit inspector: Now box, reply, Open Pane, Looks Good, "Steer this Work", Stages, Agents, Helpers. No per-agent action bar. | Read-only. Title, status tag, Now box, Asked, Recent output (capture), Where. **No actions.** |
| **Settings** | 9 tabs: General, Notifications, Menu Bar, Accounts, Ask, Agents, Automations, Fleet (Hosts / Dispatch), Advanced. In-app language override. | 2 sections: Account, Access. The admin dialogs are **English only**. | Form sections: Paired with, scope text, Notifications, Sign out. |
| **Offline / partial / signed-out** | No disconnected message on Home (buttons just disable). An unreadable remote host shows **nothing**. Partial decisions get an orange lane label. The status bar shows Disconnected + Retry. There is no sign-in. | Read failure → orange lane note with the **raw English error**. Offline host → raw enum state (`auth_failed`). 401 → `/welcome`. Viewer → "Home needs an operator". The `retry` string exists but is unused. | Red problem label, verbatim server text. Partial read shows per-host lines. Unpaired → PairingView. 401 → forget plus message. No retry (pull-to-refresh). |
| **Notifications** | `UNUserNotification` categories `tend.agent(.limited/.input)`. Titles such as "%@ asked you", "%@ needs your decision". Actions Open / Reply… / Mark as Read / Switch Account…. Menu bar four-room mark plus chips. Dock badge. | **None.** No service worker or Notification API. Only the tab title `(N)`, badges, and the mark. | APNs. Title "Tend: %@ needs you". Categories `TEND_REVIEW` (Looks good / Send back), `TEND_QUESTION` (Answer), `TEND_OPEN`, `TEND_WAITING`. Opt-in card. |

## 1. Vocabulary differences (same concept, different words)

✱ marks a divergence. KO values were checked directly against both xcstrings files and `HS` where marked "verified".

| Concept | Mac EN / KO | Web EN / KO | iOS EN / KO |
|---|---|---|---|
| Close a live question without answering | "Dismiss" / "닫기" | ✱ "Decline…" / "거절…" (⋯ menu) | ✱ "Decline" / "거절" (composer) |
| Dismiss the result report after Close all | "Dismiss" / "닫기" | ✱ "Dismiss" / **"확인"** (`HS:477`, verified) | "Dismiss" / "닫기" |
| Answer placeholder | "Your answer" / **"답변"** | "Your answer" / "답변" | ✱ "Your answer" / **"답"** (verified) |
| Finished-result approval | "Looks Good" / "확인 완료" (error text ✱ "좋아요를 기록하지 못했습니다") | "Looks Good" / "확인 완료" | ✱ "Looks good" / **"좋습니다"**, push only, and it means *Accept a review decision* |
| Return a result for rework | "Send Back" / "다시 시키기" (unit); "Return…" / "돌려보내기…" (decision) | same split | ✱ "Send back" / "돌려보내기" (push) = Return; no "Send Back"/"다시 시키기" |
| Past outcome, returned | "Returned" / "반려함" | ✱ "Returned" / **"돌려보냄"** (`HS:493`, verified) | "Returned" / "반려함" |
| Past outcome, answered | "Answered" / "답변 완료" | ✱ "Answered" / **"답함"** (`HS:491`) | "Answered" / "답변 완료" (inline outcome "Answered." / "답했습니다.") |
| Past outcome, declined / withdrawn | "거절됨" / "철회됨" | ✱ "거절함" / "철회함" | "거절됨" / "철회됨" |
| Cause: session ended | "Session ended" / "세션이 종료되었습니다" | ✱ "세션 종료" (`HS:467`, verified) | "세션이 종료되었습니다" |
| Cause: pane gone | "Pane gone" / ✱ **"패인 없음"** | ✱ "패인 없음" | **"페인 없음"** |
| Pane in general | "페인" mostly; "창" in "Choose in Pane"/"창에서 고르기" ✱ | "pane"/"Pane" (Latin, `HS:626`) ✱, "패인", "페인" mixed | "페인" |
| Your turn / Now / Last result (the inspector "now" box) | "Needs your decision" / "결정이 필요합니다", "Now" / "지금", "Last result" / "마지막 결과" | same | ✱ "NEEDS YOU" / **"내 차례"** (uppercased), "NOW", "LAST RESULT" |
| Lane-state status words | "Your turn" / "Working" / "Finished" → "내 차례" / "작업 중" / "완료" | same | same, plus "Planned" / "계획됨" (backlog) |
| Finished vs Done | both "완료" | both "완료" | both "완료" |
| Rate limited | "Rate limited" / "한도 초과"; "Limited" / "제한됨"; "Rate limited · …" / "한도 도달" ✱ | "Rate limited" / "한도 초과"; flag "Rate limited · the limit policy will resume it" | "Limited" / "제한됨" ("Rate limited" not in the catalog) |
| Host state: version skew | "Version skew" / "버전 불일치" | ✱ raw `version_skew` | ✱ "Needs update" / "업데이트 필요" |
| Host state: auth failed | "Authentication failed" / "인증 실패" | ✱ raw `auth_failed` | ✱ "Sign-in failed" / "로그인 실패" |
| Review sheet history | "History" / "기록" ↔ "Back to Review" / ✱ "검토로 돌아가기" | (same sheet strings) | n/a |
| Review mode "Inbox" | "Inbox" / "Inbox" (untranslated) | "Inbox" (KO fallback to EN) | n/a |
| Explore rail names | "Explore", "Inbox", "Ask", "Shells" left in English; "Files" / "파일" | "EXPLORE"; switch "Panes \| Messages" / ✱ "Pane \| 메시지" | ✱ "Hosts" / "호스트" (no Explore) |
| Start | "Start Work" / "Work 시작" (sheet), "Start Agent" | "Start Work" header → dialog button ✱ "Start" / "시작" | "Start Work" / "Work 시작" → ✱ "Start" / "시작" |
| Decide elsewhere | "Decide on the Dashboard" (✱ EN under KO, missing) / "Open on the Dashboard" / "대시보드에서 열기" | "Decide on that host" / "그 호스트에서 결정하세요"; "Observe only: answer on that host" / "보기 전용: 그 호스트에서 답하세요" | "Decide this on %@; its tend cannot take decisions from here." / "%@에서 결정하세요. …" |
| Asker fallback | — | ✱ literal "an agent" (not localized, `HB:209`) | "an agent" / "에이전트" (`ReviewItem.swift`) |
| "by X" on decided items | — | "by X" / KO drops "by" (`HS:497`) | "by %@" / ✱ "결정: %@" |
| Checks headline | "Checks tend ran itself…" (help text missing from the catalog) | ✱ English only: "tend ran P/N checks passed · X ago" (`needs-model.mjs:73`) | shared "tend ran %lld checks: %lld passed" (`DecisionModels.swift:476`) |
| Lane hint "Decide or review" | "결정하거나 점검할 것"; the Mac also uses "리뷰" and "검토" for review ✱ | "결정하거나 점검할 것" | same |

## 2. Design tokens

**Source.** `apps/shared/Design/tokens.json` → `scripts/design-tokens.mjs` → `crates/tend/src/dashboard/web/tokens.css` (`TOK:1`, "Generated … Do not edit") and `apps/shared/Sources/TendDesignTokens.swift`. Both apps compile the Swift file from `../shared/Sources`. `crates/tend/tests-js/design-tokens.test.mjs` guards staleness. The `mac` block of tokens.json is emitted to Swift only (`TendDesignTokens.Mac`), never to CSS.

**What each surface actually reads**:
- **Web:** all of `color`, `state`, `brand`, `host`, `font`, `size` and `metric`, through CSS variables.
- **Mac:**
  - From tokens: `Mac.*` type roles (page, cardTitle, section, label, meta, mono), `Mac.Space`, `Mac.Radius`, surfaces, inks and state inks, plus shared `Metric` row/section/status/activity/control/panel.
  - System fonts, not token roles: `TendType.body` / `detail` / `meta` are `Font.body` / `.callout` / `.subheadline` (`TendTheme.swift:13-22`).
  - System colours, not token hues: lane dots, stripes and stage segments use SwiftUI `.orange` / `.blue` / `.green` (`FleetHomeCards.swift:33-38`).
  - Unused: `cardYourTurn`, `inset`, `insetBorder`, `stripeWidth`, `cardGap`, `laneGap`, `inspectorWidth` and `inspectorDockMin`, with literals hard-coded instead.
- **iOS:**
  - From tokens: 15 colour tokens only (orange, blue, green, red, purple, mint, accent, editor, lane, sideBar, border, inputBackground, inputBorder, inset, insetBorder). Grep finds no use of `Size`, `Metric`, `Mac`, `Brand` or `host`.
  - Everything else is hard-coded in `Theme.swift`.

| Item | tokens.json shared (= web CSS) | Mac | iOS |
|---|---|---|---|
| Card radius | `card-radius` 8 | `Mac.Radius.card` 10 (some views 8/9/6 literal) | 10 (`Theme.swift:71`) |
| Lane radius | `lane-radius` 9 | `Mac.Radius.lane` 12 | 12 (`:72`) |
| Control radius | `control-radius` 4 (button 5) | `Mac.Radius.control` 6 (buttons) | 8 (`:73`) |
| Inset radius / padding | 6 / 9 | literal | 7 / 10 (`Cards.swift:329`) |
| Card padding | 12 (web phone 14) | Mac.Space.m 12 | 14 (`Theme.swift:384`) |
| Card gap | 8 | literal 8 | 10 (`Home.swift:233`) |
| Your-turn stripe | `stripe-width` 5 px | 3 pt | none (orange 45% stroke instead) |
| Secondary text | `text-secondary` rgba(0,0,0,.5) / rgba(255,255,255,.55) | `Mac.inkSecondary` .55 / .58 | **#5C5C5C / #B0B0B0** (hard-coded, `Theme.swift:85`) |
| Tertiary text | `text-tertiary` .26 / .25 | `Mac.inkTertiary` .45 / .42 | **#6E6E6E / #8E8E8E** (`:88`) |
| Primary text | `text` .85 / .85 | `Mac.inkLabel` .86 / .88 | system label |
| Surfaces (light / dark) | editor #FFFFFF/#1F1F1F, lane #F8F8F8/#252526, side-bar #F8F8F8/#181818 | `Mac.surface*`: content #FFFFFF/#1C1C1E, lane #F4F4F6/#232325, card #FFFFFF/#2C2C2F, sidebar #EFEFF2/#232326 | shared editor (cards), lane (lanes / iPhone bg), sideBar |
| State hues | orange #FF8D28/#FF9230, blue #0088FF/#0091FF, green #34C759/#30D158, red #FF383C/#FF4245, purple #CB30E0/#DB34F2, mint #00C8B3/#00DAC3 | tokens for agent dots and status words (state inks #A85400… for text); **system** `.orange/.blue/.green` for lane tints | tokens (same values); no state-ink (text) variants |
| Your-turn card fill | `card-your-turn` #FFF6EB/#33291C | unused | unused |
| Type: page / screen title | `size-title` 17 (phone 20) | `Mac.page` 20 semibold | `.title2.semibold` (Dynamic Type, about 22) |
| Type: card title | `size-card-title` 14.5 | `Mac.cardTitle` 14 semibold | `.body.semibold` (about 17) |
| Type: body / detail / meta | 13 / 12 / 11 (phone 15 / 14 / 12.5) | 13 / 12 / 11 (system styles) | `.subheadline` / `.subheadline` / `.footnote` (about 15 / 15 / 13) |
| Section label | — | `Mac.section` 13 semibold | `.caption.semibold`, uppercased, tracking 0.4 |
| Buttons | 26 / 22 px (phone min 36 / 32) | 26 / 22 pt, padding 10 / 8 | min height 44, padding h14 |
| Host discs | `host.gradients` ×8, FNV-1a % 8 | from tokens | same 8 pairs **hard-coded** (`Theme.swift:271`) |
| Brand mark lit colours | waiting #FFAA2B, error #FF5A3C, done #3FBF84, working = `brand-ink` #16181D/#F2F3F5 | same hues, hard-coded in `BrandMark.swift:52-54`; ink = **pure .black/.white**, not `brand.ink` | same hues, hard-coded (`Theme.swift:114-117`); ink = #16181D/#F2F3F5 |
| Brand mark dim | `brand.dim` 0.22 / 0.26 | 0.22 / 0.26 (`MenuBarAgentRenderer.swift:49`) | 0.22 / 0.26 (`Mark.swift:37-44`) |
| Brand mark use | header status mark, Rule A from board counts (`H:1823-1838`) | menu-bar status item, rooms lit by fleet agent states; `.quiet` when tendd failed | **only** the Home all-clear (44 pt) and the pairing header. Not a live status mark. |

## 3. Card anatomy per item kind

| Kind | Mac (emphasis) | Web (emphasis) | iOS |
|---|---|---|---|
| **Question / review decision** | ① kind row (icon + "Question"/"Review", spinner) → ② title (≤4) → ③ inset [checks, detail, recommendation, evidence, options/actions/composer] → ④ failure → ⑤ meta **asker · host · repo … time (right)**. 3pt orange stripe. | ① kind row (icon + kind, Sending…, **⋯ menu**) → ② title (≤4) → ③ inset [checks → detail → rec → evidence → options → buttons] → ④ result line → ⑤ meta asker · host (plain text) · project · time. | ① **header row: kind · asker · host badge (remote only) … time** → ② verification checks → ③ title (≤6) → ④ detail (≤5) → ⑤ recommendation → ⑥ evidence chips → ⑦ actions → ⑧ failure. **Time and asker are at the top, and checks come before the title.** No stripe; orange 45% stroke. |
| **Plain / compact decision** | header row (icon, kind, asker, host if remote, time) → checks → title → account → actions | same order (`H:476-487`) | n/a (single layout) |
| **Working unit** | [Helper badge] title (≤3) [From-lane capsule] → stage bar + stage name + done/total → inset headline (≤2) + rate-limit note → meta: status word · agent logos · host badges · repo · helper count … time | [Helper pill] title (**2-line clamp**) → stagebar + stage line → inset headline + flag → meta: state word · logos (≤3) · host badge · project · time | [Helper chip] title (2) → StageBar → InsetBox headline (2) → meta: status · logos · ≤2 host glyphs + names · repo · helpers … time; narrow width drops host names, then project |
| **Your-turn unit** | headline body-medium, ≤4 lines, plus "%lld agents wait on you"; orange stripe; no buttons | orange card plus tinted fill (`card-your-turn`); headline e.g. "Waits for your answer. Open to see the question." | headline 4 lines, medium; orange 45% stroke |
| **Done unit** | stage bar only if >1 stage; green "Finished"; Looks Good only in the inspector or the Review sheet | stagebar omitted when single-stage; title at 82% opacity; green stripe | same omission; title at 82%; green "Finished" |
| **Decided (history)** | inside the Review sheet "History" | title → outcome inset → meta ["Finished", kind icon, "by X", host, time] | **outcome chip … time (top)** → title → answer / note → host · "by %@" |
| **Agent card** (no unit) | n/a on Home (one unit per agent) | fallback `buildBoard` = one unit per agent | fallback lanes: title → status capsule bar + maker → headline → meta [logo · repo · host … time] |
| **Backlog** | column card with "Start" | title + "P<n>" → line → meta → **Start** | title + "P<n>" → line → meta; **no Start** |
| **Compact row** | dot · [question icon] · title · [moved arrow] · host · time | dot · [icon] · title · host · time | none (no compact layout) |

## 4. How strings are localized

| | Mac | Web | iOS |
|---|---|---|---|
| Mechanism | `String(localized:)`, SwiftUI `LocalizedStringKey` literals, `Text(verbatim:)` for data | one JS module, `home-strings.mjs` (`EN`, `KO = {...EN, …}`, 367 keys); HTML `data-t*` attributes swapped by `translate()`; `Intl.RelativeTimeFormat` | same as Mac |
| Catalog | `apps/tend-macos/Resources/Localizable.xcstrings`, 2369 keys; `Scripts/l10n-sync.py` | `HS`; landing and welcome have their own EN/KO tables in `landing.mjs` | `apps/tend-ios/Resources/Localizable.xcstrings`, 291 keys, all with ko; manual keys `PUSH_TITLE` / `PUSH_BODY` used by the server's APNs `title-loc-key` / `loc-key` |
| Language choice | system, with an in-app override (`TendLanguageSettings.swift` writes `AppleLanguages`: system / en / ko) | `navigator.languages[0]` starts with `ko`; no override on Home | system only |
| Gaps | 5 EN keys missing from the catalog ("Decide on the Dashboard", its help text, "Decide this result on the dashboard.", "tendd names no dashboard…", "Checks tend ran itself…") | KO falls back to EN for `connected`, `checksPassed`, `checksFailed`, `workId`, `workIdHelp`, `rvModeInbox`, `workFilter`, `fieldId`. Not localized at all: checks headline, "an agent", error messages, raw host states, `access.mode`, operators-admin and sharing-admin dialogs, share.mjs. | none missing; server-supplied problem text shown verbatim |
| Sharing | Shared Swift in `apps/shared/Sources` (DecisionModels, ReviewItem: causes, "Asked by %@", "tend ran …") compiles into both apps, but **each app resolves it against its own catalog**. 135 EN keys overlap between Mac and iOS; translations are separate copies, and at least "Pane gone" and "Your answer" differ. | **No sharing.** `HS:1-3` says the words follow the Mac xcstrings, but values are copied by hand and no test ties them together. | as Mac |
| Server text | n/a (computes its own) | server `Text` keys (`status`, `reply_ready`, `asks_to_choose`, `waits_for_answer`, `stage_is`, `waiting_on`, …) resolved by `textOf()` in `HS` | the same keys resolved in `Board.swift:39-76` against the iOS catalog. **Two separate implementations** of the server key → string mapping. |

---


# Appendix A

## Tend for Mac — surface audit (apps/tend-macos/Sources, wave15 worktree)

Paths are relative to `apps/tend-macos/Sources/` unless noted. KO values come from `Resources/Localizable.xcstrings` (2369 keys, `sourceLanguage: en`; every key there has a `ko` entry). "MISSING" means the EN key is **not in the catalog at all**, so it shows in English under KO.

### 1. Home (Fleet Home)

**Data source: the app computes the board itself.**
- `AppModel.swift:1009` polls `client.executionSnapshot()` (IPC `kind: "fleet_snapshot"`, `TendIPC.swift:1434-1441`, needs `fleet_v1`), then `rebuildWorkspaceProjections()` (`AppModel.swift:1039-1080`) runs `FleetBoard.homeAgents` + `snapshot.workGroups(pipelineRuns:)` + `FleetBoard.works(groups:agents:now:)` off the main thread and pushes the result into `FleetHomeBoard.setWorks` (`FleetHomeBoard.swift:114`).
- Lanes, helper folding, ordering, headlines and stage state all come from `FleetBoard` (`FleetHome.swift:151-644`). It does **not** render tendd's `/api/home` projection. A parity contract keeps it in line with tendd's `crates/tend/src/home_board.rs`: `apps/shared/Fixtures/home-board-parity*.json` and `Tests/HomeBoardParityTests.swift`. The fixtures README says "Not compared yet: decisions, the backlog, headlines, and limit-policy coverage".
- Decisions are a separate poll, `FleetDecisionFeed`, which runs only while Home is visible (`FleetHomeView.swift:129-152`):
  - local `decision_queue` (needs `decision_queue_v1`), falling back to `review_list` (`FleetDecisionViews.swift:131-165`, `TendIPC.swift:1572`);
  - other hosts' `decision_queue` through a fleet operation (`loadRemote`, `FleetDecisionViews.swift:96-120`);
  - merged with `model.operatorMessages` (collaboration mailbox human requests) in `decisions(_:)` (`:298-316`).
- Backlog (`work_backlog`, `FleetBacklog.swift`) and Queue (`work_queue_list`, `FleetQueueSection.swift`) are separate polls too, local host only.
- When the window is off screen, new units and decisions are held back and the clock stops (`FleetHomeBoard.swift:28-46, 93-123`).

**Lanes** (`FleetLane`, `FleetHome.swift:10-24`; tints in `FleetHomeCards.swift:33-65`):

| Lane | EN title | KO | Header hint (help/a11y only, `FleetHomeView.swift:718-724`) | Card status word (`statusLabel`) | Tint / SF symbol | Empty text (`:726-732`) |
|---|---|---|---|---|---|---|
| needsYou | "Your turn" | "내 차례" | "Decide or review" / "결정하거나 점검할 것" | "Your turn" / "내 차례" | .orange / hand.raised.fill | "Nothing needs you right now." / "지금 결정할 것이 없습니다." |
| running | "In progress" | "진행 중" | "Agents are on it" / "에이전트가 진행 중" | "Working" / "작업 중" | .blue / circle.dotted.circle | "No agent is working." / "작업 중인 에이전트가 없습니다." |
| finished | "Finished" | "완료" | "Ready for the next instruction" / "다음 지시를 기다림" | "Finished" / "완료" | .green / checkmark.circle.fill | "Nothing finished yet." / "아직 끝난 Work가 없습니다." |

- **Columns, left to right** (`FleetHomeView.swift:584-611`). Optional "Backlog"/"백로그" column (subtitle "Planned, not started"/"계획만 있고 시작 전"). Optional "Queued"/"대기열" column ("Goes when its target is ready"/"대상이 준비되면 보냅니다"). Then the 3 lanes.
- **Lane header** (`:740-760`): a 7pt dot, the title, and the count `works.count + decisions.count` (`:747`). The count:
  - includes cards folded behind the empty, helper and older rows;
  - excludes reviewed-hidden units and the Can't be answered group.
- **Rail badge:** the sidebar "Home" badge counts only `homeWorks` with `lane == .needsYou`, so tendd decisions are not in it (`WorkbenchChromeInputs.swift:110`).
- **Order** (`FleetHome.swift:360-377`):
  - Your turn: longest wait first, by `since`.
  - In progress and Finished: latest change first.
  - Your turn mixes decisions and units with `FleetBoard.yourTurn` (`:735-748`). tendd's own "system" asks go last.
- **Grouping** by project, only when the lane has more than one group (`FleetHomeView.swift:783-806`). `FleetProjectHeader` (`:880-902`) shows a folder icon, the name ("No project"/"프로젝트 없음") and a count. The system group is labelled "From tend itself"/"tend가 직접 연 항목" with a gearshape icon.
- **Folding.** Lanes other than Your turn fold cards behind ghost buttons (`:811-848`), each a show/hide toggle:
  - "%lld empty sessions"/"빈 세션 %lld개" ↔ "Hide empty sessions"/"빈 세션 숨기기";
  - "%lld possible helper sessions"/"보조 세션 후보 %lld개" ↔ "Hide possible helpers"/"보조 세션 후보 숨기기";
  - Finished only, older than 24h (`recentFinish`, `FleetHome.swift:702`): "%lld older"/"이전 항목 %lld개" ↔ "Hide older"/"이전 항목 숨기기";
  - "Show %lld reviewed"/"확인한 항목 %lld개 보기" ↔ "Hide reviewed"/"확인한 항목 숨기기".
  - Helper sessions fold *into* their root unit (`attach`, `FleetHome.swift:333-358`):
    - a helper that is asking pulls the unit into Your turn, with the headline "owner: question";
    - a helper that is working keeps a finished unit in progress, with the headline "Waiting on %@: %@"/"%@ 기다리는 중: %@".
- **Header toolbar** (`FleetHomeView.swift:198-221`), in order:
  1. "Home"/"홈" (TendType.page)
  2. Card layout segmented control: "Status emphasis"/"상태 강조" and "Compact list"/"간결한 목록"
  3. "Review"/"리뷰" with a count badge (99+) and ⇧⌘R
  4. Filter menu: "Filter"/"필터", with "All hosts"/"모든 호스트", "All projects"/"모든 프로젝트", "Clear Filters"/"필터 지우기"
  5. "Hosts %lld/%lld"/"호스트 %1$lld/%2$lld": a green dot when all hosts are online, orange otherwise. It opens a popover listing hosts and their states, plus "Leftovers…"/"남은 것…".
  6. Automation button ("Automatic: on"/"자동 처리: 켜짐" etc.)
  7. Primary "Start Work"/"Work 시작" (play.fill), disabled when disconnected or while a start is in flight.
- **Inspector:** a 400pt panel docked at the right. Below 1040pt window width it floats over the board with a shadow (`:412-430`).
- **Lane moves:** a "From %@"/"%@에서 옴" capsule and a tinted 2pt border for 4s (`FleetHomeCards.swift:162-178, 205-211`). The board animates with a spring (0.5/0.86).

### 2. Question card (FleetDecisionCard, `FleetDecisionViews.swift:751-1108`)

**Source.** A `FleetDecision` with `source .humanRequest` or `.review`. Answering a human request calls `replyCollaboration` (`collaboration_reply`); Dismiss goes the same way (`:326-360`).

**Header.**
- "Question"/"질문" (questionmark.bubble.fill) or "Review"/"리뷰" (checklist), in orange state ink.
- "Asker not listed"/"질문한 에이전트 목록에 없음", with help text "The agent that asked is not listed on its host; it may have exited. Tend folds the question once it is sure the asker is gone."

**Option rows** (`FleetDecisionOptionRow`, `:1110-1156`). Each row shows its number and full text. A click fills the composer ("Answer with this option; you see it before it is sent").

**Actions** (`otherActions`, `:1050-1069`):

| Kind | Buttons, in order | KO |
|---|---|---|
| humanRequest, no options | "Answer…" (secondary), "Dismiss" (ghost; secondary when the asker is unlisted) | "답하기…", "닫기" |
| humanRequest with options | option rows, then "Other…" (ghost), "Dismiss" | "직접 입력…", "닫기" |
| composer (human) | field "Your answer"; "Cancel" (ghost), "Send" (primary, ⌘↩) | "답변"; "취소", "보내기" |
| context menu | "Open Agent", "Decide on the Dashboard" or "Withdraw" (review only), "Dismiss Question" | "에이전트 열기", MISSING, "철회", "질문 닫기" |

- "Dismiss" has help "Close this question without answering it"/"답하지 않고 이 질문을 닫습니다".
- **Keep:** there is no "keep" on a live question. Keep exists only in the unanswerable group.
- **Asked in a pane:** a question from an agent in its pane (a `FleetWork` in Your turn, not a decision) is answered in the inspector, not on the card. `FleetReplyField` (`FleetHomeView.swift:1178-1260`) offers:
  - placeholder "Your answer"/"답변" and button "Answer"/"답하기";
  - "Sent"/"보냄" on success;
  - a stale draft warning, "Written for an earlier question from %@. It was not sent."/"%@의 이전 질문에 쓴 답입니다. 보내지 않았습니다.", with "Use for %@"/"%@에게 보내기로 바꾸기" and "Discard"/"버리기".
  - The pane button is "Choose in Pane"/"창에서 고르기", "Answer in Pane"/"페인에서 답하기", or "Open Pane"/"페인 열기" (`:1149-1153`).
  - "Always Approve This Command…"/"이 명령은 항상 승인…" appears for `waiting_choice`.

**"Can't be answered" group** (`FleetDecisionUnanswerable.swift`):
- **Data.** Decisions with `isUnanswerable` (tendd `subject_check_v1`), split from the rest at `FleetHomeView.swift:325`. The count uses tendd's exact per-host counts where they exist (`unanswerableCounts`, `FleetCantAnswer.count`, `:40-51`).
- **Placement.** The group sits at the end of Your turn, outside the lane count, collapsed by default (`:85-89`).
  - Toggle label: "Can't be answered (%lld)"/"답할 수 없는 질문 (%lld)". Help and expanded caption: "The agents that asked these are gone, so nobody will read an answer. They stay open until you close them."/"질문한 에이전트가 사라져서 답을 읽을 에이전트가 없습니다. 직접 닫기 전까지는 열려 있습니다."
- **Each row** shows its title, then a reason line built from `cause.text` (`shared/Sources/DecisionModels.swift:261-268`) and "since %@"/"%@부터". Causes: "Session ended"/"세션이 종료되었습니다", "Replaced by another agent"/"다른 에이전트로 교체", "Pane gone"/"패인 없음", "Agent exited"/"에이전트 종료", "Asker gone"/"질문한 에이전트 없음". Then asker · host · time.
- **Row action:**
  - local host: "Keep"/"유지" (help "Put it back in Your turn"/"내 차례로 되돌립니다", IPC `decision_keep`);
  - other host: "Open on the Dashboard"/"대시보드에서 열기".
- **Group action:** "Close All…"/"모두 닫기…" (only with `close_unanswerable_v1`, local rows only).
  - Confirmation alert: "Close %lld questions?"/"질문 %lld개를 닫을까요?" with message "Each asking agent gets a reply that you dismissed it. This can't be undone."/"질문한 에이전트는 각각 사용자가 질문을 닫았다는 회신을 받습니다. 되돌릴 수 없습니다.", and buttons "Close %lld"/"%lld개 닫기" (destructive) and "Cancel".
  - Result: "Closed %lld questions."/"질문 %lld개를 닫았습니다." plus "Skipped %lld: %@", with a "Dismiss" button.
  - Overflow: "%lld more not listed"/"목록에 없는 질문이 %lld개 더 있습니다".
  - Remote note: "Other hosts' questions are kept or closed on the dashboard." or "tendd names no dashboard, so other hosts' questions are kept or closed on those hosts."

### 3. Review item / result

There are two separate flows.

**(a) A tendd review item** (`source .review`, decided with `review_dispose`, `TendIPC.swift:1615`), on the same card:
- buttons "Accept"/"수락" (secondary), then "Return…"/"돌려보내기…" (ghost);
- composer field "Note for the asker"/"요청자에게 남길 메모", with "Cancel", "Accept" (note optional) and "Return" (primary, needs text)/"돌려보내기" (`:1088-1094`);
- "Withdraw"/"철회" in the context menu only.
- **Another host's item:** the card shows "Decide on the Dashboard" (**MISSING** in the catalog, `:1015`) with help "Open this result on the dashboard's Needs you page, and decide it there" (**MISSING**). Without a dashboard it shows text "tendd names no dashboard, so this result is decided on %@…" (**MISSING**). The feed error "Decide this result on the dashboard." is also **MISSING** (`:412`).
- **Evidence:**
  - `FleetVerificationChecks` comes first: seal icon, headline, per-check ✓/✗, tail in monoSmall. Its help "Checks tend ran itself, before the agent's own account" is **MISSING**.
  - The asker's account follows (detail, recommendation 💡 "What the asker recommends"/"요청자가 권하는 선택"), then `FleetEvidenceChips` (`:634-700`).
  - Chips: tests are green, red or secondary; links use an accent capsule. A chip opens Changes, a URL, or Finder.
  - Unavailable reasons: "Not a revision Tend can read", "On %@; Changes reads this Mac's checkouts", "No checkout recorded", "Not a web address", "Not an absolute path"; test results "Passed"/"통과", "Failed"/"실패", "Result not recorded".

**(b) A finished unit "Looks Good".**
- Inspector: a ghost "Looks Good"/"확인 완료" (checkmark) at the right, help "Set this aside until it changes again"/"다시 바뀔 때까지 보드에서 치워 둡니다" (`FleetHomeView.swift:1114-1121`).
- It records `review_record` (`TendIPC.swift:1631`, `review_record_v1`), or falls back to a local UserDefaults mark `fleetHome.reviewed` (`FleetHome.swift:888-935`).
- **Send Back:** the reply field placeholder is "Next instruction"/"다음 지시" and the button is "Send Back"/"다시 시키기".

**Review sheet** (`FleetReviewView.swift`, ⇧⌘R). The queue is Finished units the person asked for, not reviewed, within a range (`FleetReview.swift:97-111`).
- **Header** (`:103-145`): "Review"/"리뷰" with a count.
  - Mode: "Cards"/"카드", "Inbox"/"Inbox", "Digest"/"요약".
  - Range: "Today"/"오늘", "24 hours"/"24시간", "This week"/"이번 주", "All"/"전체".
  - "History"/"기록" ↔ "Back to Review"/"검토로 돌아가기" (title "Decided"/"결정한 것"), and "Done"/"완료".
- **Above the queue:** a "Decisions"/"결정" block, "oldest first"/"오래된 순", with decision cards.
- **Detail blocks** (`FleetReviewDetail`, `:267-328`): title, then "Asked"/"맡긴 일", "Recorded commands"/"기록된 명령" (FleetEvidenceSection), "Agent's report"/"에이전트의 보고".
- **Buttons** (`:331-355`): "Later"/"나중에" [H], "Send Back"/"다시 시키기" [R], "Open"/"열기" [O], spacer, primary "Looks Good"/"확인 완료" [E]. Hint line: "E Looks Good   H Later   R Send Back   O Open".
- **Inbox:** "Mark All Reviewed…"/"모두 확인 완료…".
- **Digest:** "Reply to All…"/"모두에게 답장…", "Mark Project Reviewed"/"프로젝트 모두 확인", "Summarize"/"요약하기" (uses the Ask provider), "Reply to %lld…", "Mark Reviewed".
- **Caught up:** "All caught up"/"모두 확인했습니다".

### 4. Start work

- **Start Work sheet** (`WorkConsoleViews.swift:4`, opened by `AppModel.presentWorkStart`, `AppModel.swift:1149`; IPC `work_up`, `TendIPC.swift:1344-1365`).
  - Title "Start Work"/"Work 시작", subtitle "Say what needs to get done. The agents work out the rest."/"해야 할 일만 적으세요. 나머지는 에이전트가 판단합니다."
  - Task field placeholder: "What needs to get done? For example: fix the login timeout…".
  - Sections: "Where"/"위치" (Host picker, folder "Choose…"), "Team"/"팀" (pipeline/template), "Plan"/"플랜" (after a dry run), "More options"/"추가 옵션" (Work ID, Workspace, External issue, Additional context, "Plan only — do not create agents"/"플랜만 만들기 — 에이전트를 생성하지 않음").
  - Footer: "Cancel", "Add to Queue"/"대기열에 추가" (only with `work_queue_v1`), primary "Start Work" or "Build Plan"/"플랜 만들기" (↩).
  - With no routing it shows "No Work routing is configured yet…" and "Configure Work…".
- **New Agent sheet** (`NewAgentView.swift`, 560×640; IPC `agent_start_v1`).
  - Title "New Agent"/"새 에이전트", subtitle "Start one coding agent in a folder; it opens as a tab."
  - Sections: "Agent", "Where", "Task" (first prompt optional), role.
  - Placement segmented control: "Split pane"/"페인 분할", "New window"/"새 윈도우", "New session"/"새 세션", "Tend terminal"/"Tend 터미널".
  - Toggle "Use as default for ⌥⌘T". Buttons "Cancel" and "Start Agent"/"에이전트 시작" (↩, and ⌘↩ anywhere).
- **Other entry points:**
  - Home empty board: "Start Work" + "New Agent".
  - Backlog column "Start"/"시작" (`tend work up`).
  - Queue column "Start Now"/"지금 시작".
  - Menu bar "Start Work…".
  - App menu "Start Tend Work…", "New Default Agent", "New Agent…" (`TendApp.swift:282-297`).
  - The separate "Fleet Dispatch" sheet (`FleetDispatchViews.swift`) is not localized-checked here.

### 5. Explore equivalent (sidebar)

- **Activity rail** (`SidebarViews.swift:976-1030`): "Home"/"홈" (house, needs-you badge), a divider, then `railModes` = Explore, Inbox, Ask, Shells, Files (`AppModel.swift:33`). "Work" exists as a mode but is not on the rail.
  - Titles: "Explore"/"Explore", "Inbox"/"Inbox", "Ask"/"Ask", "Shells"/"Shells", "Files"/"파일". Only Files is translated.
- **Data:** the same `executionSnapshot.watchHosts` (fleet_snapshot) as the rest of the app.
- **Explore list** (`:836-910`):
  - Status scope: "All"/"전체", "Attention"/"주의 필요", "Active"/"활성".
  - Grouping: "Host tree"/"호스트 트리", "Status groups"/"상태 그룹", "No groups"/"그룹 없음".
  - Sort: "Topology"/"구조", "Latest activity"/"최근 활동", "My latest prompt"/"내 최근 프롬프트", "Agent latest update"/"에이전트 최신 업데이트".
  - Host grouping pins "Needs attention · N"/"주의 필요" (max 5, "Show all %lld") above "Execution topology"/"실행 구조", whose tree runs host › session › window › pane.
  - Status buckets: "Needs attention", "Rate limited"/"한도 초과", "Active"/"활성", "Idle agents"/"대기 중인 에이전트", "Shell panes"/"셸 페인". "All panes · %lld"/"전체 페인 · %lld".
- **Empty states** (`WorkbenchEmptyStates.swift:32-70`):
  - "No panes detected"/"감지된 페인 없음" with "Start Agent" and "New Shell";
  - "No fleet hosts"/"플릿 호스트 없음" with "Register SSH Host…";
  - "No matching panes"/"일치하는 페인 없음".
- **Other modes:** Inbox shows "Operator"/"오퍼레이터" and "Needs attention", empty "Nothing needs attention". Shells shows "Native shells".

### 6. Agent detail (`DetailViews.swift:366-551`, FleetAgentDetailView)

- **Header**, in order:
  1. 12pt status dot (`status.color`)
  2. title: aiTitle, else `@alias`, else kind
  3. status word, or the rate-limit badge
  4. session id (mono)
  5. on the right: `TendAgentActionBar`, "Open in Live Watch"/"Live Watch에서 열기" (only when the agent does not need attention), module menu
- **Actions** (`TendAgentActions.swift:120-145`). Primary first, then the "Agent Actions"/"에이전트 작업" ellipsis menu:
  - limited: "Switch Account and Resume…"/"계정 전환 후 재개…";
  - needsInput or error: "Send “continue”"/"“continue” 보내기" and "Move to Another Account…"/"다른 계정으로 옮기기…";
  - other states: "Move to Another Account…".
- **When the agent needs attention:** `InboxAgentRequestCard` sits above the tabs.
- **Tabs:** "Summary"/"요약", "Conversation"/"대화", "Shell"/"셸". Shell is hidden when there is no pane.
  - Summary: Markdown, or "No retained summary"/"보관된 요약 없음". Then a GroupBox "Execution location"/"실행 위치" with Host, Work, Backend, Session, Window, Pane, Directory, Runtime, Model.
  - Conversation: "Request"/"요청" and "Response"/"응답".
  - Shell: `PaneCaptureView`, or "No shell binding"/"셸 연결 없음".
- **Quick look:** Home's card click opens `FleetQuickLookView` ("This agent is no longer running.", "Bottom of the pane", "Live", "No output yet.") and the full view from there.

### 7. Settings (`TendSettingsView.swift:29-107, 172-205`)

TabView (toolbar tabs), in order:

| # | rawValue | EN | KO | SF symbol | Sections seen |
|---|---|---|---|---|---|
| 1 | general | "General" | "일반" | gearshape | Appearance/모양 (Theme, "Translucent sidebar"), Startup/시작, Home/홈 ("Card layout", "Animate cards moving between lanes"), Keyboard shortcut/키보드 단축키, Default project folder/기본 프로젝트 폴더 (`:698-760`) |
| 2 | notifications | "Notifications" | "알림" | bell.badge | macOS permission/macOS 권한; toggles "Notify when an agent needs you", "Notify when an agent finishes a turn", "Play a sound", "Show the count on the Dock icon", "Notify before a usage limit runs out" (`AgentAttentionViews.swift:168-193`); tendd notifier "Post a desktop notification when an agent needs you", "Delivery"; Collaboration/협업 (`BehaviourSettingsView.swift`) |
| 3 | menuBar | "Menu Bar" | "메뉴 막대" | menubar.rectangle | "Group activity" (By agent / By Work / By Fleet host), "Maximum activity icons" |
| 4 | accounts | "Accounts" | "계정" | person.2 | "When an agent hits its usage limit" |
| 5 | ask | "Ask" | "Ask" | bubble.left.and.text.bubble.right | Providers/프로바이더 |
| 6 | agents | "Agents" | "에이전트" | person.3.sequence | (TendAgentsSettingsPane), "Not editable from here" |
| 7 | automations | "Automations" | "자동화" | bolt.badge.automatic | Run log; rule editor: Rule, Event, Filters, Timing, Ask condition, Fixed action, Guards |
| 8 | fleet | "Fleet" | "플릿" | server.rack | segmented "Hosts"/"호스트" and "Dispatch"/"작업 배치" |
| 9 | advanced | "Advanced" | "고급" | slider.horizontal.3 | Daemon/데몬, Configuration file/설정 파일 |

- **Legacy tabs** map to current ones (`:93-97`): providers → ask, behaviour → notifications, modules → accounts, dispatch → fleet, runtime → advanced.
- **Every tab** shares a bottom `TendRestartBar`.

### 8. Empty and error states

| Condition | Where | Text (EN / KO) |
|---|---|---|
| Nothing at all (no works, decisions, backlog, queue, problems) | `FleetEmptyBoard`, `FleetHomeView.swift:1348-1374` | "No work yet"/"아직 Work가 없습니다"; "Start a Work from a template, or start one agent. Each shows up here as a unit of work."; buttons "Start Work" and "New Agent". When disconnected the buttons are disabled; there is no separate "disconnected" message on Home. |
| Empty lane | lane body | see §1 table |
| Partial decision read | lane-top orange label (`:763-770`) | "Some decisions could not be read: %@"/"일부 결정을 읽지 못했습니다: %@"; "Showing the first decisions on this Mac; more are waiting."; "Could not read decisions: %@"/"결정을 읽지 못했습니다: %@" |
| Remote host unreadable | `loadRemote` | no message: "A host that cannot be read now shows nothing to act on" (`FleetDecisionViews.swift:104`) |
| Backlog/queue failure | own column | "Could not read the backlog: %@"; "Could not read the queue: %@"; "tendd could not save the queue; nothing new goes until it can" |
| Looks Good not recordable | feed problem | "tendd cannot record reviews; update tend and restart tendd"; "Could not record Looks Good: %@" → KO "좋아요를 기록하지 못했습니다: %@" (button itself is "확인 완료") |
| Host offline | Hosts button dot (orange) + popover | host states "Online"/"온라인", "Offline"/"오프라인", "Connecting"/"연결 중", "Degraded"/"성능 저하", "Version skew"/"버전 불일치", "Authentication failed"/"인증 실패", "Disabled"/"비활성" (`ContentView.swift:583-594`); decision answer "%@ is not connected."/"%@에 연결되어 있지 않습니다." |
| Daemon down | status bar (`WorkbenchChromeViews.swift:480-510`) + sidebar status | "Connecting to tendd…", "tendd connected", "tendd upgrade required" + "Use Bundled tendd", "Disconnected"/"연결 끊김" + "Retry"/"다시 시도"; menu-bar mark goes `.quiet` on failed/upgradeRequired (`MenuBarAgentController.swift:660-663`) |
| Signed out | — | no sign-in concept in the Mac app; other hosts' items are decided "on the dashboard, under its sign-in" (`FleetDecisionViews.swift:1011`) |
| Agent moved on | reply send | "The agent moved on. Open its pane to see where it is." |

### 9. Notifications

- **Posting:** `TendAgentAttentionCenter` (`AgentAttentionCenter.swift`) builds notifications; `AgentNotifications.swift:160-180` posts them.
  - Content fields: title, subtitle, body (≤160 chars, whitespace collapsed, `:91-97`). `threadIdentifier` is the pane key. Sound is set when the "Play a sound" preference is on.
  - The pane being looked at is skipped; repeats within 30s are suppressed.
- **Categories and actions** (`AgentNotifications.swift:97-139`):
  - `tend.agent.limited`: "Switch Account…"/"계정 전환…", "Open"/"열기", "Mark as Read"/"읽음으로 표시";
  - `tend.agent`: Open, Mark as Read;
  - `tend.agent.input` (waiting_input/asked): Open, "Reply…"/"답변…" (text input, button "Send"/"보내기", placeholder "Message for the agent"/"에이전트에게 보낼 메시지"), Mark as Read.
- **Agent titles** (`AgentAttentionCenter.swift:580-640`). `name` is `@alias`, else aiTitle, else kind. Subtitle is "host · session › window".

| Event | Title EN | KO | Body |
|---|---|---|---|
| waiting_choice | "%@ is waiting for a choice" | "%@이(가) 선택을 기다리는 중" | lastNotification, else lastResponse |
| error/failed | "%@ hit an error" | "%@에서 오류 발생" | same |
| blocked | "%@ is blocked" | "%@ 막힘" | same |
| asked | "%@ asked you" | "%@이(가) 물어봤습니다" | the question |
| other attention | "%@ needs your input" | "%@이(가) 입력을 기다리는 중" | same |
| finished | "%@ finished" | "%@ 작업 완료" | lastResponse, else recap |
| limited | "%@ hit its usage limit" | "%@이(가) 사용 한도에 도달했습니다" | "Resets at %@." / "…Switch it to another account to carry on now." |
| human request | "%@ needs your decision" | "%@에서 확인을 요청합니다" | request body; subtitle host · session · window (`:394`) |
| reply failed | "Your reply was not sent" | "답변을 보내지 못했습니다" | the text; subtitle the reason |
| usage alert | "%@ has used %lld%% of its %@" ("5-hour limit"/"weekly limit") | "%1$@ %3$@의 %2$lld%%를 썼습니다" | "Resets %@." (`TendUsageAlerts.swift:191-198`) |

- **Menu bar.** One `NSStatusItem` (`MenuBarAgentController.swift:88`) draws the brand mark plus agent chips, with a "%lld more items" overflow.
  - Popover: "Tend", "All agents"/"전체 에이전트", "Unread"/"읽지 않음", "Host disconnected", "Send Reply"/"답장 보내기", "Discard Draft", "Open Agent", "Mark as Read", "Accounts and usage"/"계정 및 사용량", "Open Tend", "Start Work…", "Open Live Watch", "New Shell", "Settings…", "Quit".
  - Empty popover: "No agents need attention"/"확인이 필요한 에이전트가 없습니다". Grouping options: "By agent", "By Work", "By Fleet host".
- **Dock badge:** `dockCount`, when "Show the count on the Dock icon" is on.

### 10. Card anatomy (emphasis layout unless noted)

| Kind | Order, top to bottom |
|---|---|
| **Question / review decision** (`FleetDecisionViews.swift:863-922`) | ① kind row: icon + "Question"/"Review", spinner (orange state ink, label 11 medium) → ② title (cardTitle 14 semibold, ≤4 lines) → ③ inset box: verification checks, detail (≤3), recommendation, evidence chips, option rows/actions or composer → ④ failure (red) → ⑤ meta row: asker · host · "· repo" · "Asker not listed" … relative time (right). Frame: card surface, 3pt orange stripe at the left. |
| **Plain decision** (compact panel) (`:838-860`) | header row (icon, kind, asker, host if remote, time) → checks → title (section 13) → account → actions → failure; 8pt radius, orange 0.45 border |
| **Working unit** (`FleetHomeCards.swift:145-251`) | ① ["Helper"/"보조" badge if orphan] title (cardTitle, ≤3) [From-lane capsule] → ② stage bar (segments + current stage name + done/total) → ③ inset: headline (body, secondary ink, ≤2), rate-limit note → ④ meta: status word ("Working", blue ink, symbol) · agent logos · host badges · "· repo" · helper count (person.2) … relative time |
| **Your-turn unit** | same, but headline is body-medium, label ink, ≤4 lines, plus "%lld agents wait on you"/"에이전트 %lld개가 답을 기다림"; orange stripe. No buttons on the card; tap opens the inspector |
| **Done unit** | stage bar only if more than one stage (`:181`); green "Finished" word; time = lastChange; Looks Good lives in the inspector/Review |
| **Unit with helpers** | helper count `Label("%lld", person.2)` in meta (help "%lld helper sessions"/"보조 세션 %lld개"); inspector adds a "Helpers"/"보조 세션" section |
| **Compact row** (`:258-325`) | 8pt tint dot · [questionmark.bubble for a decision] · title (body medium, 1 line) · [moved arrow] · host (meta, ≤110pt) · relative time (tertiary); min height 30, radius 6 |
| **Inspector** (`FleetHomeView.swift:992-1170`) | title (page 20) + "repo · hosts", ✕ → "now" box (lane word "Needs your decision"/"결정이 필요합니다", "Now"/"지금", "Last result"/"마지막 결과"; headline; asked context; choice panel or reply field; "Also waiting on you"/"함께 답을 기다리는 에이전트"; buttons [pane button] [Work Details] [Always Approve…] … [Looks Good]); tint 0.1 fill, radius 9 → "Stages"/"단계" → "Agents"/"에이전트" (status tag, logo, owner, status, "Open") → "Helpers" |

### 11. Localization mechanics

- **Mechanisms.** `String(localized:)` is used in model and computed labels. SwiftUI literal `Text("…")`/`Button("…")` resolves as a `LocalizedStringKey`; `LocalizedStringKey` is also the type of the `title` properties on `TendSettingsTab` and the DetailTab.
- **Verbatim text.** `Text(verbatim:)` is used for user and agent data.
- **Catalog.** One catalog, `Resources/Localizable.xcstrings`, built with `SWIFT_EMIT_LOC_STRINGS: YES` and `LOCALIZATION_PREFERS_STRING_CATALOGS: YES`. `Scripts/l10n-sync.py` merges an `xcodebuild -exportLocalizations` export into it (`project.yml:50-55`).
- **In-app language override:** `TendLanguageSettings.swift` writes `AppleLanguages`, with options system, en, ko.
- **Shared code.** `apps/shared/Sources` (DecisionModels, ReviewItem; 18 `localized` uses) is compiled into both the Mac and iOS apps. Each app resolves those strings against its **own** catalog. iOS has a separate `apps/tend-ios/Resources/Localizable.xcstrings` (291 keys).
  - 135 keys are shared; 3 differ in KO: "Pane gone" is "패인 없음" on Mac and "페인 없음" on iOS; "Your answer" is "답변" on Mac and "답" on iOS; "Skipped %lld: %@" uses positional specifiers only on iOS.
  - The iOS catalog has no "Looks Good" or "Send Back".
  - No strings are shared with the web dashboard.
- **Missing from the catalog** (EN shows under KO): "Decide on the Dashboard" (`FleetDecisionViews.swift:828,1015`), "Open this result on the dashboard's Needs you page, and decide it there", "Decide this result on the dashboard.", "tendd names no dashboard, so this result is decided on %@. Turn on tend's dashboard to decide it from here.", "Checks tend ran itself, before the agent's own account".
- **Terms that diverge in the Mac KO catalog:**
  - Pane is "페인" in most strings but "패인" in "Pane gone" and "창" in "Choose in Pane"/"창에서 고르기" and "The agent moved on…"/"창을 열어".
  - "Review" is "리뷰", but "Back to Review" is "검토로 돌아가기" and "Inspect" is "점검".
  - "Looks Good" is "확인 완료", but its error says "좋아요를 기록하지 못했습니다".
  - "Finished", "Done" and "Done" (stage) all translate to "완료".
  - Limits appear as "Rate limited" "한도 초과", "Limited" "제한됨", and "Rate limited · …" "한도 도달".
  - "Explore", "Inbox", "Ask" and "Shells" are left in English.

### 12. Design

- **Token flow.** `TendDesignTokens.swift` is generated (`scripts/design-tokens.mjs`) from `apps/shared/Design/tokens.json` and compiled in via `project.yml:30` (`../shared/Sources`).
  - `TendDesignSystem.swift` maps `TendDesignTokens.Mac.*` to the type, spacing, radius, surface and ink roles.
  - `TendTheme.swift` maps the shared surface tokens and `Metric.*` (row 24, section header 32, status bar 22, activity bar 46, control radius 4, panel radius 6).
- **Type roles** (`TendDesignTokens.Mac`, `:115-128`):

| Role | Value |
|---|---|
| page | 20 semibold |
| cardTitle | 14 semibold |
| section | 13 semibold |
| body | 13 regular |
| secondary | 12 regular |
| label | 11 medium |
| meta | 11 regular, tabular |
| mono | 12 |
| monoSmall | 11 |
| glyph | 9 semibold |
| icon | 16 |
| emptyIcon | 28 |
| heroIcon | 40 |
| displayIcon | 60 |

  - `TendType.body`, `detail` and `meta` themselves are system `Font.body`, `.callout` and `.subheadline` (13/12/11, `TendTheme.swift:13-22`), not the token roles. `TendType.page`, `cardTitle`, `section`, `label`, `tabularMeta` and `mono` come from tokens.
  - Shared `Size` (title 17, cardTitle 14.5, inspectorTitle 16, now 13.5…) is used only by the menu bar and settings.
- **Spacing** (4pt scale): xxs 2, xs 4, s 8, m 12, l 16, xl 20, xxl 24, xxxl 32.
- **Radii.** Mac: control 6, group 8, card 10, lane 12. Shared `Metric`: card 8, lane 9, option 6, choice 7, inset 6.
  - Hard-coded values in views: decision plain card 8, inspector box 9, compact row 6, unanswerable row 8.
  - Unused in Sources: `cardYourTurn`, `inset`, `insetBorder`, `stripeWidth` (5; the actual stripe is 3pt), `insetPadding`, `cardGap`, `laneGap`, `inspectorWidth`, `inspectorDockMin`. The views hard-code 8/12/16/400/1040 instead.
- **Surfaces** (light/dark):
  - Mac: sidebar #EFEFF2/#232326, content #FFFFFF/#1C1C1E, lane #F4F4F6/#232325, card #FFFFFF/#2C2C2F.
  - Shared: editor #FFFFFF/#1F1F1F, border #E5E5E5/#2B2B2B.
- **Inks:** label 86%/88%, secondary 55%/58%, tertiary 45%/42%, hairline 8%, control 5%/8%, track 16%/20%.
- **State inks** (text, 4.5:1): orange #A85400/#FF9F2E, green #16743A/#32D35F, blue #0066D6/#409CFF, red #C8302E/#FF5A5F, purple #8E3FC0/#C77DFF.
- **Fills:** accentFill #0066D6/#0A6CDA, danger #C8302E/#D13A36.
- **Raw hues** (dots, segments): orange #FF8D28, blue #0088FF, green #34C759, red #FF383C, purple #CB30E0, mint #00C8B3.
- **System colours in the board.** Lane tint dots, stripes and stage segments use SwiftUI system `.orange`, `.blue` and `.green` (`FleetHomeCards.swift:33-38`, `FleetHomeView.swift:955-962`), not the token hues. The status words use `TendInk.state`.
- **Agent status colours** (`AgentStatus.swift:122-131`): error red, needsInput/asked orange, limited purple, working blue, idle mint, done green.
- **Buttons** (`TendTheme.swift:213-280`): primary, secondary and ghost.
  - Height 26 (22 small), horizontal padding 10 (8 small), radius `TendRadius.control` 6.
  - Body font semibold for primary, medium otherwise.
  - Disabled opacity 0.45.
- **Brand mark** (`BrandMark.swift`; geometry from `assets/brand/tend/mark-rooms.svg`): one square in a 64-unit grid, split by a canted passage into 4 rooms. Each room lights if any fleet agent is in its state:

| Room | Position | Lit by | Colour |
|---|---|---|---|
| waiting | top-right, small | needsInput, asked, limited not covered by policy | #FFAA2B |
| error | top-left | error | #FF5A3C |
| working | bottom-right, large | working | ink (black/white), not a hue |
| done | bottom-left | done | #3FBF84 |

  - Dim rooms are drawn in ink at alpha 0.22 (light) or 0.26 (dark) (`MenuBarAgentRenderer.swift:49`). The mark is 16×16 in the 24pt-high status image.
  - The renderer uses pure `.black`/`.white` ink rather than the token `Brand.ink` (#16181D/#F2F3F5). The colours are hard-coded in `BrandMark.swift:52-54` and match the `Brand` tokens.
  - The mark is `.quiet` (all rooms dim) when tendd has failed or needs an upgrade.

# Appendix B

## Web dashboard (tendd) — surface audit

Repo `/Users/june/personal/tend-wt-wave15` @ `da4593db`. Read-only. Abbreviations, all under `crates/tend/src/`:
`H` = dashboard/web/home.mjs, `HB` = dashboard/web/home-board.mjs, `HS` = dashboard/web/home-strings.mjs,
`HTML` = dashboard/web/home.html, `CSS` = dashboard/web/home.css, `TOK` = dashboard/web/tokens.css,
`EX` = dashboard/web/explore.mjs, `EXM` = dashboard/web/explore-model.mjs, `NM` = dashboard/web/needs-model.mjs,
`SRV` = dashboard/server.rs, `HR` = dashboard/home.rs, `BOARD` = home_board.rs.
(There is no `home-model.mjs`; the board model is `home-board.mjs`.)

### 0. Shell, routes, data flow

- One page for `/`, `/home`, `/needs`, `/explore`, `/explore/messages`, `/settings` (`assets.rs:50-53`, `SRV:510`; client route table `H:2093-2100`). `/welcome` is a separate page (`welcome.html`, landing.mjs).
- Rail: Home / Explore / Settings (`HTML:17-21`); Explore and Settings are hidden unless `/api/access` answered (`H:1891-1892`).
- Poll every 5 s (`REFRESH_MS`, `H:20`), each read times out at 8 s (`H:27`), one poll in flight (`H:2348`), also on `visibilitychange` (`H:2585`).
- Poll (`H:2363-2468`): `GET /api/home` + `GET /api/decisions/history?closures=true` + `GET /api/access` in parallel (`H:2386-2391`). If `/api/home` succeeded, `/api/fleet` is read only when on Explore (`H:2398`). If `/api/home` failed (old daemon, Windows proxy, paired device), fallback reads `/api/decisions` + `/api/fleet` (`H:2401`).
- **Board selection** `currentBoard()` (`H:2471-2479`): `state.data.server` present → `fromServerBoard(server, {history,…})` (`HB:594`); else `buildBoard({decisions, history, fleet})` (`HB:410`), a full client-side port of the Mac rules (one agent = one unit, "phase 1" per `HB:9-13`).
- Read-only (viewer / `public_read` / auth none): board not drawn; `buildBoard({fleet})` only feeds the header and statusbar (`H:1899-1915`).
- `/api/device/home` and `/api/device/fleet` exist (`dashboard/devices/routes.rs:132-133`, `devices/operate.rs:1043`) for paired iOS devices; **the web never calls them**.

### Does the web render the server projection?
Partly. With `/api/home` it takes the server's units, stages, focus, helpers, titles/headlines (as `Text` keys), `probable_helper`/`empty_session`/`recent`/`reviewed` flags, `backlog`, `projects`, `hosts`, `agent_count` (`HB:529-566`, `HB:594-664`). But it then:
- **flattens** server groups and **re-groups** by project client-side (`HB:598-605` → `groupByProject`, `HB:398-408`);
- **recomputes counts**: `count = visible.length - folds.reviewed.length` (`HB:643`); the server's `lanes.*.count` is ignored;
- **applies folds** client-side (helpers → empty → older → reviewed; Your turn never folds) (`HB:632-641`);
- re-derives decision cards from the raw `entry` with the client's own `decisionUnit()` (`HB:568-590`, reusing `HB:193-248`);
- **appends decision history** (`/api/decisions/history`) to Finished client-side, unsorted after server units (`HB:614-622`);
- builds the Can't-be-answered group client-side (`HB:339-362`, `HB:649`);
- builds host/lane notes client-side from `decision_hosts` (`HB:605-613`). Server's `unanswerable_note`, `covers`, `generated_at`, `reviewed_marks` are not read by the web (grep: no hits).

### 1. Home — lanes, grouping, counts, folding

| Lane key (client / server) | EN title · sub · empty | KO title · sub · empty |
|---|---|---|
| `backlog` / `backlog[]` (shown only if non-empty, `H:1878`) | "Backlog" · "Planned, not started" | "백로그" · "계획만 있고 시작 전" |
| `needsYou` / `needs_you` | "Your turn" · "Decide or review" · "Nothing needs you right now." | "내 차례" · "결정하거나 점검할 것" · "지금 결정할 것이 없습니다." |
| `running` / `running` | "In progress" · "Agents are on it" · "No agent is working." | "진행 중" · "에이전트가 진행 중" · "작업 중인 에이전트가 없습니다." |
| `finished` / `finished` | "Finished" · "Ready for the next instruction" · "Nothing finished yet." | "완료" · "다음 지시를 기다림" · "아직 끝난 Work가 없습니다." |

(`HS:8-16`, `HS:398-406`, `HS:137-138`; lane map `H:600-604`.) Desktop: lanes are equal grid columns (`CSS:146`). ≤760 px: one lane at a time behind segments "Backlog / Your turn / In progress / Finished" with counts (`HTML:62-67`, `CSS:367-373`, `CSS:541`).

- Lane head DOM: `h2` title, `.count` number, `.sub` subtitle (`H:619`).
- Grouping: by project, project order = first unit's position; "No project" (`""`) / "프로젝트 없음"; tend's own asks last under "From tend itself" / "tend가 직접 연 항목" with server icon (`HB:398-408`, `H:633-640`). Project header shown only when a lane has >1 group (`H:631`), with per-group count.
- Sorting (fallback path only): Your turn oldest-wait first, decision before unit on tie; running/finished newest first (`HB:466-468`). Server path keeps server order.
- Folds (button text, EN / KO; `H:657-674`, `HS:149-154`, `HS:529-534`):
  - helpers: "N possible helper sessions" / "Hide possible helpers" · "보조 세션 후보 N개" / "보조 세션 후보 숨기기"
  - empty: "N empty sessions" / "Hide empty sessions" · "빈 세션 N개" / "빈 세션 숨기기"
  - older (>24 h, `FINISHED_WINDOW_MS` `HB:17`): "N older" / "Hide older" · "이전 항목 N개" / "이전 항목 숨기기"
  - reviewed (Finished, server only): "Show N reviewed" / "Hide reviewed" · "확인한 항목 N개 보기" / "확인한 항목 숨기기"
- Counts surface: lane head, phone segments (`H:1822`), rail badge = Your-turn count, "99+" cap (`H:1818-1821`), `document.title` "(N) Home · tend" (`H:1839`).
- Header controls (`HTML:39-59`): layout toggle "Status emphasis"/"Compact list" ("상태 강조"/"간결한 목록"); "Review" button with badge (server board only, `H:868`); Filter popover with Hosts + Project selects, "All hosts"/"All projects", "Clear Filters" ("모든 호스트"/"모든 프로젝트"/"필터 지우기"); Hosts popover "Hosts on/all" ("호스트 on/all") listing each host's dot + alias + raw state string (`H:1770-1782`); primary "Start Work" ("Work 시작").
- Lane notes (quiet, orange text, `H:606-612`): `"<host>: Observe only: answer on that host"`, `"<host>: Unavailable: <source>"` (KO "읽지 못함: …"), `"<host>: …"` for truncated queue, and in **In progress** `"<host>: <state> · <error>"` for a non-online remote host — `state` is the raw enum (e.g. `auth_failed`), not localized.

### 2. Question card (and Can't be answered)

Data: queue item `kind: "human_request"` → `decisionUnit` (`HB:196-224`): title = `decision.needed` else `body`; detail = body when needed exists; asker `@alias` | pane | `"tend"` | literal `"an agent"` (English, `HB:209`); `actionable = local || control`.

Actions (emphasis layout, `H:397-443`), in DOM order:
1. Numbered option rows (each opens the composer prefilled with the option; tooltip "Answer with this option; you see it before it is sent" / "이 선택지로 답합니다. 보내기 전에 확인합니다").
2. If options: ghost "Other…" / "직접 입력…"; else "Answer…" / "답하기…".
3. Composer: textarea placeholder "Your answer"/"답변"; buttons **Cancel** · **Send** ("취소" · "보내기"); ⌘/Ctrl+Enter sends, Esc closes (`H:262-316`).
4. Overflow "⋯" (aria "More"/"더 보기") → danger "Decline…" ("거절…") → composer with **Cancel** · **Decline** (`H:436-439`, `H:280-282`).
- POST `/api/decisions/requests/{id}` body `{answer}` or `{answer, declined:true}`, plus `host` for remote (`H:233-251`, `H:277`). Result line: "Answered"/"답함", "Declined"/"거절함".
- Not actionable: flag "Observe only: answer on that host" / "보기 전용: 그 호스트에서 답하세요" (`H:400-403`). No write auth: no actions at all (`H:404`).

**Can't be answered** (folded group at the bottom of Your turn, `H:683-731`): toggle "Can't be answered (N)" / "답할 수 없는 질문 (N)", collapsed by default; help "The agents that asked these are gone, so nobody will read an answer. They stay open until you close them." / "질문한 에이전트가 사라져서 …". Excluded from Your-turn count (`HB:629`, `BOARD:1803-1806`).
- Row (`H:745-769`): title → why line `"<cause> · since <time>"` (causes EN "Session ended" / "Replaced by another agent" / "Pane gone" / "Agent exited" / "Asker gone"; KO "세션 종료" / "다른 에이전트로 교체" / "패인 없음" / "에이전트 종료" / "질문한 에이전트 없음") → meta [asker, host, when, **Keep** button].
- **Keep** ("유지", tooltip "Put it back in Your turn") → POST `/api/decisions/keep` `{request_id, episode[, host]}` (`HB:376-379`), result "Back in Your turn" / "내 차례로 되돌렸습니다". Shown only if host `subject_check`.
- **Close all** ("모두 닫기"; per host "Close all on <host>") → inline confirm "Close N questions[ on host]? Each asking agent gets a reply that you dismissed it." + "This can't be undone." with **Cancel** · danger **Close N** ("N개 닫기") → POST `/api/decisions/close-unanswerable` `{items:[{request_id,episode}], by_label:"web"[, host]}` (`HB:382-386`, `H:772-795`). Report "Closed N questions." / "Skipped N: id (reason)" with **Dismiss** ("확인").
- Reasons a remote host can't: "<host> is observe-only…", "…or upgrade tend there.", "Close all is turned off on <host>; Keep still works." (`HS:72-76`).
- Finished outcome for a closure: "Closed, asker gone: <cause>" / "질문한 에이전트가 없어 닫음: …"; tend's own: "Closed by tend: <reason>" (`H:564-569`).

### 3. Review item

Two different things are called "review" on the web:

**(a) Review decision card** (queue `kind:"review"`, Your turn; `HB:226-246`). Header kind "Review"/"리뷰". Inset: checks box (headline from `NM:73`, English only: `"tend ran P/N checks passed · X ago"`, lines `"passed: …"` / `"FAILED: …"`), detail, recommendation (bulb icon), evidence chips (PR `owner/repo#N` link, commit sha7, diff range, test command pass/fail tone, path basename; `HB:146-167`), options.
- Buttons (`H:426-431`): **Accept** · ghost **Return…** ("수락" · "돌려보내기…"). Option click → composer in return mode. Composer row: **Cancel** · **Accept** · primary **Return** (note placeholder "Note for the asker"/"요청자에게 남길 메모"; Return requires text) (`H:283-291`). Overflow: danger **Withdraw** ("철회").
- POST `/api/decisions/reviews/{id}` with `{disposition:"accepted"[,note][,option]}`, `{disposition:"returned", note}`, `{disposition:"withdrawn"}`. Results "Accepted"/"수락함", "Returned"/"돌려보냄", "Withdrawn"/"철회함".
- Remote host without `decides_reviews`: flag "Decide on that host" / "그 호스트에서 결정하세요".

**(b) Review sheet** (finished Work results asked by a person, server board only; `H:850-1350`). Button "Review"+badge; dialog title "Review N"; modes "Digest"/"Cards"/"Inbox" (KO "요약"/"카드"/"Inbox" — untranslated); ranges "Today"/"24 hours"/"This week"/"All" ("오늘"/"24시간"/"이번 주"/"전체"); "Done". Queue = finished, `requested_by_person`, not reviewed, since scope start (`HB:668-684`).
- Detail: title, meta [logos, host badge, · project, · owner, helpers count, · when], block "Asked"/"맡긴 일", block "Agent's report"/"에이전트의 보고" (mini-markdown renderer `H:952`), "The rest is in the pane." when cut.
- Buttons order (`H:1035-1075`): **Later** [H] · **Send Back** [R] · **Open** [O] · spacer · primary **Looks Good** [E]. KO: "나중에" · "다시 시키기" · "열기" · "확인 완료". Keys hint string: "E Looks Good   H Later   R Send Back   O Open".
- Looks Good → POST `/api/home/reviewed` `{id, review_key}` (409 if changed; `H:1718-1739`, `HR:157-163`). Send Back opens reply composer → POST `/api/panes/{pane}/prompt` `{text, submit:true[, socket]}` then best-effort POST `/api/home/instruction` `{id, session_id, state_entered_at, kind:"send_back"|"answer", text, question}` (`H:1395-1446`). Not at prompt: "This agent is not at its prompt; open it to send it back."
- Bulk: Digest per-project "Mark Project Reviewed", checkbox selection bar "N selected · Clear · Mark Reviewed"; Inbox footer "Mark All Reviewed…" → "Mark all N reviewed?" "Mark All"/"Cancel". Empty: "All caught up" / "모두 확인했습니다".

### 4. Start work

- Header primary "Start Work" (visible iff `canWrite()`, `H:1791`) → `<dialog>`: "Work" (input, pattern `[A-Za-z0-9][A-Za-z0-9._-]*`, max 64), "What to do" textarea; **Cancel** · **Start** (`HTML:93-101`). KO: "Work" (untranslated, `workId` falls back), "할 일", "취소" · "시작".
- POST `/api/work-control/up` `{work[, body]}` (`H:2498`). Disabled with "Starting work from the dashboard is turned off on this host." when `capabilities.work_start` false or 501.
- Backlog card **Start** (play icon) → POST `/api/work-control/up` `{work, workspace?, external?}` (`H:815-822`). No template/agent/host picker; `workIdHelp` "A short name, like fix-login" is defined but unused.
- Empty board copy promises templates: "Start a Work from a template, or start one agent…" (`HS:18`) — the dialog has neither.

### 5. Explore (hosts / sessions / panes)

Data: `/api/fleet` (`H:2398`), per-agent cap attention from `/api/home` (`boardAttention`, `HB:758`). Output `GET /api/panes/{pane}/output` + SSE `/api/panes/{pane}/output/stream` (`EX:529`, `EX:544`); prompt POST `/api/panes/{pane}/prompt`, **Interrupt** POST `/api/panes/{pane}/abort` (`EX:577`, `EX:601`).
- Sidebar header "EXPLORE" + count + switch "Panes | Messages" (KO "Pane | 메시지", `HS:626-627`). Search placeholder "Filter Explore"/"Explore 필터".
- Filter menu: show "All / Attention / Active" ("전체/주의 필요/활성"); Group "Host tree"/"Status groups" ("호스트 트리"/"상태 그룹"); Order "Topology / Latest activity / My latest prompt / Agent latest update" (`EX:135-175`).
- Status groups `attention, limited, active, idle, shell` (`EXM:28`): EN "Needs attention / Rate limited / Active / Idle agents / Shell panes"; KO "주의 필요 / 한도 초과 / 활성 / 대기 중인 에이전트 / 셸 페인". Pinned "Needs attention · N" section, limit 5 with "Show all N" (`EXM:233`, `EX:268-276`).
- Row: dot (orange/purple/blue/mint/tertiary, `CSS:580-584`), product logo or icon, title, sub = state label or command (`EX:186-200`).
- Session/window detail: meta "<host> · N windows · N panes", "Details"/"Runtime Details" facts (Host, Socket, Session, Window, Index), metrics (Agents, Working, Need attention, Subagents, Child processes), "Agent reports", "Windows" cards (`EX:386-475`).
- Pane viewer: state chip, "Messages", "Share…" (if `pane_sharing`), output, composer placeholder "Type to the pane — Enter sends, Shift+Enter adds a line", **Send** · ghost danger **Interrupt** ("보내기" · "중단").
- Remote pane: "This pane is on another host: open it there, or from Tend for Mac." — no output for remote hosts.

### 6. Agent detail (inspector)

Opened by clicking a work card (`H:509-514`) or compact row. `renderInspector` (`H:1585-1680`), DOM order:
1. Head: `h2` title, sub "project · host", close ✕.
2. "Now" box, label by lane: "Needs your decision" / "Now" / "Last result" (KO "결정이 필요합니다"/"지금"/"마지막 결과"); headline text; reply composer when `replyTarget` (`HB:692-703`): Your turn + asked/waiting_input → placeholder "Your answer", button **Answer** ("답하기"); Finished idle → "Next instruction", **Send Back** ("다시 시키기"); stale draft notice "Written for an earlier question from X. It was not sent." + "Use for X" / "Discard".
3. Link "Open Pane" (or primary "Answer in Pane" when not replyable) → `/explore#pane=…` (local only).
4. Finished + server board: ghost **Looks Good**.
5. "Steer this Work" (Work source only): placeholder "Send the next instruction to every live collaborator", **Send to N** → POST `/api/work-control/prompt` `{identity:{workspace_id,work_id}, text, submit:true}` (`H:1514-1583`).
6. "Stages" timeline: ring, name, owner (logo + "kind · host"), state "Your turn / Working / Done / Waiting".
7. "Agents", "Helpers" (help "Sessions other agents started to help with this work.") rows: dot, logo, owner, status ("Limited", "Asked you", state label), "Open".
- Decision detail (compact layout only): kind header, sub "asker · host · project", "Open Agent", the full card (`H:1683-1700`). Decided detail: "Last result" + outcome + "by X" (KO drops "by": `by: (who) => \`${who}\``, `HS:497`).

### 7. Settings

`renderSettings` (`H:2146-2182`). Only two sections:
- **Account** ("계정"): rows "Signed in" (`email (role)` | "Dashboard token" | "Not signed in"), "Access mode" (raw `access.mode`), "May act", "May start work" (Yes/No · 예/아니요); read-only reason paragraph; actions "Sign in as an operator", "Sign out", "Forget token", and an "Upgrade with token" form (password field "Dashboard access token", placeholder "Token from tend init") → POST `/auth/enroll` (signed-in) or localStorage + `/api/access` check (`H:2028-2070`).
- **Access** ("접근 권한", operators only): help "Operators, viewers, paired devices and shared panes."; buttons "Manage access…" (`operators-admin.mjs`: dialog "Access" with "Devices", "Operators", "Viewers"; device code lookup/approve `/api/devices/lookup`, `/api/devices/approve`; remove/limit `/api/devices/{id}/remove|limit`, "Remove all devices"; `/api/operators/{id}/remove`; `/api/viewers` add/remove) and "Manage shares…" (`sharing-admin.mjs`: `/api/shares/status`, `/api/shares/check`, `/api/shares`, `/api/shares/{id}/revoke`).
- Both admin dialogs are **English-only hard-coded strings** (e.g. "Remove", "Limit to deciding", "No paired devices.", "Copy link", "Revoke", "Share created. Copy its link…"; `operators-admin.mjs:127-250`, `sharing-admin.mjs:26-72`); they do not import `HS`.
- No language, theme, notification, layout, host or agent settings on the web.

### 8. Empty and error states

| Situation | What shows | Where |
|---|---|---|
| Every read ok, nothing anywhere | `#empty-board`: board icon, "No work yet"/"아직 Work가 없습니다", body "Start a Work from a template, or start one agent. Each shows up here as a unit of work." | `HTML:72-76`, `H:1866-1870` |
| One lane empty | "Nothing needs you right now." etc. | `H:652` |
| Read failed for a lane | orange `.lane-note` with ⚠ + raw error message (`decisions`→Your turn, `fleet`→In progress, `history`→Finished); messages are English (`"<path> → 503"`, network text) | `H:621-626`, `H:111-118` |
| Partial queue | "<host>: …" (truncated), "<host>: Unavailable: <source>" | `HB:608-610` |
| Offline remote host | In progress note "<host>: <raw state> · <error>"; Hosts popover dot warn/bad/off + raw state; hosts dot warn when not all online | `HB:612-613`, `H:1768-1782` |
| Daemon unreachable | statusbar chip "Disconnected"/"연결 끊김" (class `failed`), `#status` "Disconnected"; brand mark rooms all unlit | `H:1808-1838`, `H:2436` |
| Connecting | "Connecting to tendd…" / "tendd에 연결 중…"; connected chip text "tendd" | `HS:62-64` |
| 401 on reads | redirect to `/welcome?return_to=…` (landing page, own EN/KO tables in `landing.mjs`, lang stored under `tend.landing.lang`) | `H:2259-2265` |
| Lost operator access | everything private dropped; `#status` = error ("sign in to see what needs you" / "signed in without operator access", English literals), button "Sign in as an operator" → `/auth/login?return_to=` | `H:2286-2297` |
| Signed-in viewer / public read | operator notice: hand icon, "Home needs an operator"/"홈은 운영자만 쓸 수 있습니다", reason text (4 variants `roAnonymous/roViewer/roViewerNoEnroll/roNoToken`), sign-in / upgrade actions; lanes, segments, filter, layout hidden | `H:1899-1950` |
| Review sheet empty | "All caught up"; range: "Nothing left to review in this range…"; all: "Every result you asked for has been reviewed." | `H:907-912` |
| Explore empty | "No panes detected" + detail, "No matching panes", "No panes", "Loading…" | `EX:242-317` |
| No "Retry" control | `retry: "Retry"` string exists but is unused (grep) | `HS:65` |

### 9. Notifications

- **No web push.** No service worker, `Notification`, `PushManager`, or `setAppBadge` anywhere in `dashboard/web` (grep). `manifest.webmanifest` is a standalone PWA manifest with icons only.
- In-page signals only: `document.title` prefix "(N) " for Your-turn count (`H:1839`), rail badge (`H:1817-1821`), Review button badge, and the four-room brand mark (`H:1823-1838`).
- Push exists server-side only for paired **iOS** devices via APNs (`dashboard/push/mod.rs:1-20`, `POST /api/device/push`); the web's only mention is the device list note "Notifications: on (…)" / "Notifications are off: <reason>" in Manage access (`operators-admin.mjs:203-205`).

### 10. Card anatomy (DOM order)

**Question / review — emphasis** (`H:446-475`), `article.card.decision.emphasis.needsYou`:
1. `.e-kind`: icon (`question-fill` | `checklist`) + "Question"/"Review" ("질문"/"리뷰") · ["Sending…"] · ⋯ menu
2. `.e-title` (agent text; clamp 4 lines for decisions, `CSS:404`)
3. `.inset`: checks box → detail → recommendation → evidence chips → options → buttons/composer
4. result line
5. `.meta` (no lane state word): asker · host (plain text, not badge) · "· project" · when (right-aligned)

**Question / review — plain** (inspector / compact detail, `H:476-487`): `.d-head` [icon, kind, asker, host if remote, when|Sending…, ⋯] → checks → `.d-title` → detail/rec/evidence → actions → result. Time is in the head here, at the end in emphasis.

**Working unit** (`H:509-546`), `button.card.work.emphasis.<lane>` (no actions on the card itself):
1. `.title-row`: ["Helper"/"보조" pill] + `.e-title` (2-line clamp)
2. `.stagebar` (one 4 px segment per stage, colored by stage state) + `.stage-line` [current stage name, "done/total" when >1]
3. `.inset`: headline (status line / `Text` key) · flag "Rate limited · the limit policy will resume it"
4. `.meta`: state word with glyph ("Your turn"/"Working"/"Finished"; KO "내 차례"/"작업 중"/"완료") · product logos (≤3) · host badge (hue disc + laptop/server icon + alias) · "· project" · when
Same function draws a **waiting agent** in Your turn (orange card, headline "Waits for your answer. Open to see the question." / "Asks you to choose…" when no message).

**Done unit** (Finished work): same as working, but stagebar + stage line **omitted when single-stage** (`H:518`), title dimmed to 82 % (`CSS:405`), green stripe.

**Decided (history) card** (`H:573-589`): `.e-title` → `.inset .outcome` [check|turn icon, "Answered/Accepted/Returned/Withdrawn/Declined[: note]"] → `.meta` ["Finished" state, kind icon, "by X", host badge, when].

**Compact row** (`H:548-561`): dot (lane tint) · [question icon] · title · host · when. All actions move to the inspector.

**Backlog card** (`H:797-847`): title + "P<n>" pill → line → meta "workspace · pipeline · external" → **Start**.

### 11. Localization

- One string table: `HS` (`EN` `HS:5-387`, `KO = {...EN, …}` `HS:394-766`, 367 keys). `languageOf(navigator.languages)` → `"ko"` iff the first language starts with `ko`, else `"en"` (`HS:777-780`, `H:21`). No user override on Home (landing has its own picker + `tend.landing.lang`).
- Static HTML labels via `data-t`, `data-t-label`, `data-t-title` swapped in `translate()` (`H:2514-2522`); `<html lang>` set.
- Relative time via `Intl.RelativeTimeFormat(lang, {style:"short"})` (`HS:783-795`).
- Server `Text` values (`{key, …args}`) resolved by `textOf()` (`HS:768-775`) against keys `status`, `background_task`, `reply_ready`, `kind_on_host`, `asks_to_choose`, `waits_for_answer`, `stage_is`, `waiting_for_agents`, `owner_says`, `waiting_on` (`HS:120-133`, `HS:510-519`).
- KO falls back to EN for: `connected` ("tendd"), `checksPassed`, `checksFailed`, `workId` ("Work"), `workIdHelp`, `rvModeInbox` ("Inbox"), `workFilter`, `fieldId` (computed by comparing tables). KO `panes` is "Pane" (Latin script, `HS:626`).
- Not localized at all: `NM` checks headline and lines (`NM:73-85`), asker fallback `"an agent"` (`HB:209`), read/write error messages (`H:116-118`, `H:133`), raw host states, `access.mode`, both admin dialogs, `share.mjs` ("Send command", "View only", "Offline. Displayed output may be stale.").
- Sharing with Mac/iOS: comment says words follow `apps/tend-macos/Resources/Localizable.xcstrings` (`HS:1-3`) but values are hand-copied; no generator or test ties them (no test references `xcstrings`). KO term drift visible: pane = "pane" / "패인" / "페인" (`HS:441`, `HS:467`, `HS:578`).
- Defined but unused: `checksPassed`, `checksFailed`, `rateLimited`, `retry`, `workIdHelp`. `review` key defined twice in EN (`HS:27`, `HS:137`).

### 12. Design

- `TOK` is generated: header `/* Generated by scripts/design-tokens.mjs from apps/shared/Design/tokens.json. Do not edit. */` (`TOK:1`); generator also writes `apps/shared/Sources/TendDesignTokens.swift` (`scripts/design-tokens.mjs:17-21`); `--check` mode + `crates/tend/tests-js/design-tokens.test.mjs` guard staleness.
- Type scale (desktop / ≤760 px override `CSS:351`): title 17/20, card title 14.5 (no phone override), inspector title 16/18, now 13.5/15.5, body 13/15, detail 12/14, meta 11/12.5, pill 10/11, badge 9. Hard-coded outliers: 15px (`.empty-board h2`, `.operator-notice h2`, `CSS:163,167`), Messages graph 10–12px (`CSS:713-744`).
- Font: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, "Apple SD Gothic Neo", "Malgun Gothic", sans-serif`; mono `ui-monospace, "SF Mono", Menlo, Consolas, "Cascadia Mono"`.
- Radii: control 4, button 5, panel 6, option 6, inset 6, choice 7, card 8, lane 9; dialog 12 and review card 14 hard-coded (`CSS:322`, `CSS:504`); pills/badges 8–10.
- Spacing: card padding 12 (14 on phone), card gap 8, lane gap 12, board padding 16, inset padding 9, stripe 5, row 24, button 26/22 (min 36/32 on phone), inspector 400 (dock ≥1040).
- State colors (light / dark): orange `#FF8D28`/`#FF9230` (your-turn, needs-input, asked), blue `#0088FF`/`#0091FF` (working, in-progress), green `#34C759`/`#30D158` (done, finished), red `#FF383C`/`#FF4245` (error), purple `#CB30E0`/`#DB34F2` (limited), mint `#00C8B3`/`#00DAC3` (idle); Your-turn card fill `#FFF6EB`/`#33291C` (`TOK:19-41`, `TOK:96-107`).
- Card emphasis: 5 px left stripe in `--tint` (blue default, orange needsYou + tinted background + 35 % orange border, green finished) (`CSS:398-401`). Stagebar 4 px segments, blue/orange/green, pending = 25 % secondary (`CSS:194-198`). Dots 7 px (`.dot` green/orange warn/red bad/tertiary off/blue connecting, `CSS:134-138`); compact dot 8 px.
- Host identity: 8 gradients `--host-0..7` chosen by FNV-1a(alias) % 8 (`HB:51-58`, `TOK:45-52`), 14 px disc.
- **State mark**: inline SVG four-room brand mark, `viewBox="4 4 56 56"`, 16×16 px, rooms `room-waiting` (top-right), `room-error` (left-top), `room-working` (right-bottom), `room-done` (left-bottom) (`HTML:26-31`). Unlit = `--brand-ink` at `--brand-dim` 0.22/0.26; lit: waiting `#FFAA2B`, error `#FF5A3C`, working = ink at full opacity, done `#3FBF84` (`CSS:80-85`). Rule A (`H:1823-1838`): waiting = Your-turn count > 0; error = any agent `state === "error"`; working = running count > 0; done = finished count > 0; all unlit when not connected.

### Server projection (`home_board.rs`) — names and text

- Purpose (`BOARD:1-19`): port of Mac `FleetBoard`; "Words the client owns (status labels, the fixed headlines) are returned as keys ([`Text::Key`])". `Text` is untagged `Plain(String)` | `Key{key, ...args}` (`BOARD:73-100`).
- Lanes enum `NeedsYou | Running | Finished` → `needs_you`, `running`, `finished` (`BOARD:386-392`); stage states `needs_you | running | done | waiting` (`BOARD:377-384`).
- Output (`BOARD:1828-1864`): `generated_at`, `hosts[]{alias,local,state,error,mode,tend_version}`, `lanes.{needs_you,running,finished}.{count,groups[{project,system,items}]}`, `backlog`, `projects`, `reviewed_marks`, `decision_hosts[]` (control, decides_reviews, covers, truncated, unavailable, subject_check, close_unanswerable, unanswerable_note, unanswerable_count), `agent_count`. Capability `home_board_v1` (`BOARD:31`).
- Item `type: "decision"` carries `id, host, local, system, repo, since, actionable, asker, entry` (raw queue entry) (`BOARD:1610-1623`) — titles for questions are left to clients. Work units carry `source` = `{kind:"work", workspace, work}` or `{kind:"agent", host, session_id}` (`BOARD:974`, `BOARD:1023`), plus `title`, `headline`, `lane`, stages, focus, agents, helpers, review fields, `review_key` (`BOARD:1508-1528`).
- Server-emitted text: keys `status` (`{status:"limited"|"asked"|<state>}`), `background_task`, `reply_ready{who}`, `kind_on_host{kind,host}`, `asks_to_choose`, `waits_for_answer`, `stage_is{stage,status}`, `waiting_for_agents`, `owner_says{owner,text}`, `waiting_on{owner,text}` (`BOARD:206-217`, `672-723`, `941-954`, `1008`, `1280-1298`). Plain strings it composes itself: `"⟳ <summary line>"` (`BOARD:672`), owner `"<kind> · <alias>"` (`BOARD:469`), Work title = metadata title or work id (`BOARD:1688`); everything else is agent-authored text cut to 160 chars (`BOARD:115-120`). No English sentences, lane names or button labels are emitted by the server.
- Grouping on the server: `grouped()` — project order by first item, system asks last as `{project:null, system:true}` (`BOARD:1627-1649`); Your turn sorted `(system, since, position)` (`BOARD:1800`); `needs_you.count` excludes `entry.unanswerable` (`BOARD:1803-1806`).
- Sources (`home_board/sources.rs:1-35`): pipeline runs, automation rules, swap health (500 ms timeout, reuse 60 s), work store/backlog, review records; built board reused 2 s (`BOARD_REUSE`).

# Appendix C

## iOS/iPadOS app audit (apps/tend-ios, wave15 worktree)

Read-only audit of `/Users/june/personal/tend-wt-wave15/apps/tend-ios/Sources` plus `apps/shared/Sources`. Paths below are relative to `apps/tend-ios/Sources` unless prefixed. KO = `ko` stringUnit in `apps/tend-ios/Resources/Localizable.xcstrings` (291 keys, sourceLanguage `en`, locales en+ko, no key missing ko).

### 0. Data sources (DashboardClient.swift)

| Endpoint | Method | Client fn (line) | Used for |
|---|---|---|---|
| `/auth/devices/request`, `/auth/devices/token` | POST | 413, 422 | pairing (device code flow) |
| `/api/device` | GET | 501 | scope + `features` (`home`, `capture`, `work`) + push support |
| `/api/device/home` | GET | 552 | **server board** (`DeviceHome`, Board.swift:337) |
| `/api/device/fleet` | GET | 542 | hosts + agents (`FleetSnapshot`, Fleet.swift:9) |
| `/api/decisions?limit=100` | GET | 443 | per-host queue (fallback Your turn; push action re-read) |
| `/api/decisions/history?limit=100&closures=true` | GET | 449 | Finished "Decided" items |
| `/api/decisions/requests/{id}` | POST `{answer, declined, host?}` | 457 | answer / decline question |
| `/api/decisions/reviews/{id}` | POST `{disposition, note?, option?, host?}` | 489 | accept / returned / withdrawn |
| `/api/decisions/keep` | POST `{request_id, episode, host?}` | 466 | Keep (Can't be answered) |
| `/api/decisions/close-unanswerable` | POST `{items[], by_label:"ios", host?}` | 476 | Close all |
| `/api/device/capture` | POST | 561 | agent recent output |
| `/api/device/work-options?host=` | GET | 572 | Start Work pickers |
| `/api/device/work` | POST | 582 | Start Work |
| `/api/device/push`, `/push/remove`, `/forget` | POST | 511, 532, 437 | push registration, sign out |

`via:` header param (`"notification"`) is passed when a push button decides (AppModel.swift:1225-1232).

**Projection vs local computation.** Two code paths (AppModel.swift:337-340, 702-735):
- `readsBoard = operates && features.contains("home")`. When true, Home renders the daemon's `home_board` projection: lanes `needs_you`/`running`/`finished` with server-ordered groups (`project`, `system`) and items (`kind:"unit"` or decision) (Board.swift:1-7, 388-442). The app only applies the Mac folds client-side (`FoldedLane`, Board.swift:470-497) and splits unanswerable questions out of Your turn (Board.swift:422-428).
- Otherwise (decide-only key, or older dashboard): Your turn = `/api/decisions` per host (`HostSectionView`, Screens.swift:10-37); In progress / Finished agents computed locally by `FleetLanes` (Fleet.swift:195-228): In progress = live agents `working|starting` and not needsAttention/awaitingReply, newest first; Finished = `idle`, not awaiting, with a prompt or response, `since` within 24 h (`finishedWindow`, Fleet.swift:197), clock skew <= 60 s. Grouped client-side by `repo` (last cwd segment) via `LaneContent.byProject` / `finishedGroups` (Home.swift:374-398).
- Finished's decided items always come from `/api/decisions/history` (Home.swift:298-328), merged after board units.
- Polling: refresh every 10 s while active; history every 6th round (Screens.swift:131-142).

### 1. Home (Home.swift, Board.swift, BoardViews.swift)

Lanes (`Lane`, Theme.swift:121-167). Backlog appears first only if `board.backlog` non-empty (AppModel.swift:815-817).

| Lane | EN title / KO | Subtitle EN / KO | Empty EN / KO | Tint |
|---|---|---|---|---|
| backlog | "Backlog" / "백로그" | "Planned, not started" / "계획만 있고 시작 전" | "Nothing planned." / "계획된 작업이 없습니다." | TendInk.secondary |
| yourTurn | "Your turn" / "내 차례" | "Decide or review" / "결정하거나 점검할 것" | "Nothing needs you right now." / "지금 결정할 것이 없습니다." | orange |
| inProgress | "In progress" / "진행 중" | "Agents are on it" / "에이전트가 진행 중" | "No agent is working." / "작업 중인 에이전트가 없습니다." | blue |
| finished | "Finished" / "완료" | "Ready for the next instruction" / "다음 지시를 기다림" | "Nothing finished yet." / "아직 끝난 Work가 없습니다." | green |

Layout:
- iPad / wide: `sizeClass == .regular && width >= 900` → `board`: HStack of `LaneColumn`s, spacing 12, padding 16 (Home.swift:21, 84-91). Column header = title (`TendType.laneTitle`) + count + right-aligned subtitle (Home.swift:185-201); column bg `TendSurface.panel`, radius `laneRadius` 12.
- iPhone / narrow (<900 pt incl. iPad portrait split): `LaneStrip` of count cells over a paged `TabView` (swipe between lanes) (Home.swift:53-80). Cell: big count (`.title2.bold`, orange if Your turn > 0), dot + lane title; selected = tint 12% fill + 60% stroke 1.5 (Home.swift:127-174). Two-row grid at `.accessibility1` (or `.xxLarge` with 4 lanes) (Home.swift:101-103). Lane subtitle + `HostsSummary` ("Hosts %lld/%lld" / "호스트 %1$lld/%2$lld") shown at top of each page (Home.swift:234-242).
- Decide-only: In progress count replaced by a lock icon, a11y "Not shown on this device" / "이 기기에서는 보이지 않음" (Home.swift:135-139).
- Toolbar: iPad leading `HostsSummary`; trailing "Start Work" / "Work 시작" (`play.fill`) when `canStartWork` (Home.swift:31-45).
- Tab badge = `actionableCount` (decidable pending only) (Screens.swift:156, AppModel.swift:468).

Counts (`AppModel.count`, AppModel.swift:825-841): board path = `FoldedLane.count` = all lane items minus reviewed units; Finished adds `past.count` (all history, not only 24h). Fallback: pending.count / inProgress.count / recentPast+finishedAgents.

Grouping: board groups shown with header only when lane has >1 group (BoardViews.swift:98, 124). Header: `folder` + project name + count (Cards.swift:74-88); nil project → "No project" / "프로젝트 없음"; `system` group → `gearshape` + "From tend itself" / "tend가 직접 연 항목" (BoardViews.swift:11-22). Finished decided items: `checkmark.seal` + "Decided" / "결정한 것" + count (Home.swift:638-651).

Folding (outside Your turn only, Board.swift:479-496; buttons BoardViews.swift:128-131), order of precedence reviewed > probableHelper > emptySession > !recent:

| Fold | Show EN / KO | Hide EN / KO |
|---|---|---|
| helpers | "%lld possible helper sessions" / "보조 세션 후보 %lld개" | "Hide possible helpers" / "보조 세션 후보 숨기기" |
| empty | "%lld empty sessions" / "빈 세션 %lld개" | "Hide empty sessions" / "빈 세션 숨기기" |
| older | "%lld older" / "이전 항목 %lld개" | "Hide older" / "이전 항목 숨기기" |
| reviewed | "Show %lld reviewed" / "확인한 항목 %lld개 보기" | "Hide reviewed" / "확인한 항목 숨기기" |
| older decided (history) | "%lld older decided" / "이전에 결정한 것 %lld개" | "Hide older" |

Fold buttons are plain `.borderless` text buttons, collapsed by default, revealed units appended below (no group headers). Backlog lane footer: "Planned Work starts from Tend for Mac or the dashboard; here you can start new work." / "계획된 Work는 Mac용 Tend나 대시보드에서 시작합니다. 여기서는 새 작업을 시작할 수 있습니다." (Home.swift:290). Backlog cards have no Start (Cards.swift:484-513).

Your turn order (board path, BoardViews.swift:62-103): all-clear empty state with TendMark → per-host unavailable/truncated lines → coverage-unknown warning → undecoded warning → groups → `CantAnswerGroup`. Notices above any lane: green `outcome` label (Your turn only) and red `problem` label (Home.swift:268-281); `NotificationOffer` card in Your turn (Home.swift:244).

### 2. Question card (DecisionCard.swift) and Can't be answered (CantAnswer.swift)

Header kinds (DecisionCard.swift:88-123), all orange: question "Question"/"질문" (`questionmark.bubble.fill`); result "Review"/"리뷰" (`checklist`); waiting "Waiting"/"대기 중" (`hand.raised.fill`).

Question actions (non-composing, DecisionCard.swift:126-147): numbered `OptionRow`s (1..n, tap → composer prefilled with the option text); then one button "Answer…"/"답하기…" (no options) or "Other…"/"직접 입력…" (with options). Composer (174-213): TextField placeholder "Your answer"/"답"; buttons in order "Cancel"/"취소" (left), spacer, "Decline"/"거절" (card style), "Send"/"보내기" (primary, filled). Both disabled when empty. POST `/api/decisions/requests/{id}` with `declined` false/true. Outcome text "Answered."/"답했습니다." or "Declined."/"거절했습니다." (AppModel.swift:941). No separate close/dismiss for an answerable question on the card.

Non-decidable: label `desktopcomputer` "Decide this on %@; its tend cannot take decisions from here." / "%@에서 결정하세요. 그 호스트의 tend는 여기서 보낸 결정을 받을 수 없습니다."; waiting: "Waiting in its pane; answer it on a computer." / "페인에서 기다리는 중입니다. 컴퓨터에서 답하세요." (81-86).

Can't be answered group (CantAnswer.swift):
- Disclosure row: chevron + "Can't be answered (%lld)" / "답할 수 없는 질문 (%lld)", collapsed by default, a11y "Expanded"/"펼쳐짐" "Collapsed"/"접힘" (40-57). Count = max(listed, sum of hosts' `unanswerable_count`) (Board.swift:448-450). Not counted in badge or Your turn count.
- Expanded: explainer "The agents that asked these are gone, so nobody will read an answer. They stay open until you close them." / "질문한 에이전트가 사라져서 답을 읽을 에이전트가 없습니다. 직접 닫기 전까지는 열려 있습니다." (89); one button per closable host "Close All…"/"모두 닫기…" or "Close All on %@…"/"%@에서 모두 닫기…" (96); host notes (observe-only/upgrade/closeOff, AppModel.swift:1275-1277); `CloseAllReport`; rows; "%lld more not listed" / "목록에 없는 질문이 %lld개 더 있습니다".
- Confirm dialog title "Close %lld questions?"/"질문 %lld개를 닫을까요?" (or "... on %@?"), destructive "Close %lld"/"%lld개 닫기", "Cancel"; message "Each asking agent gets a reply that you dismissed it. This can't be undone." / "질문한 에이전트는 각각 사용자가 질문을 닫았다는 회신을 받습니다. 되돌릴 수 없습니다." (60-72). POST `/api/decisions/close-unanswerable`.
- Row (`CantAnswerRow`, 144-192): title (3 lines) → reason line `person.crop.circle.badge.xmark` e.g. "Pane gone · since 3 days ago" (cause KO: "Session ended"/"세션이 종료되었습니다", "Replaced by another agent"/"다른 에이전트로 교체", "Pane gone"/"페인 없음", "Agent exited"/"에이전트 종료", "Asker gone"/"질문한 에이전트 없음"; "since %@"/"%@부터") → meta asker · host · time … "Keep"/"유지" (hint "Put it back in Your turn"/"내 차례로 되돌립니다"). Opacity 0.92. POST `/api/decisions/keep`; outcome "Back in Your turn."/"내 차례로 되돌렸습니다."
- Result report: "Closed 1 question."/"질문 1개를 닫았습니다.", "Closed %lld questions.", "Skipped %lld: %@"/"%1$lld개는 건너뛰었습니다: %2$@", with "Dismiss"/"닫기" (119-140).

### 3. Review item (result kind)

Card order (DecisionCard.swift:21-66): header → `VerificationChecks` (green/red seal per check, name or command) → title (`decision.needed` else summary, 6 lines) → detail (summary, 5 lines) → recommendation (`lightbulb.fill` orange) → `EvidenceList` chips → actions → failure line.

Actions row (148-166), order: options as numbered rows (tap → composer with "Accept with: %@"/"다음으로 수락: %@"); "Accept"/"수락" (card, immediate POST disposition `accepted`); "Return…"/"돌려보내기…" (opens composer); spacer; `ellipsis` Menu (a11y "More"/"더 보기") containing destructive "Withdraw"/"철회" (POST `withdrawn`). Composer: placeholder "Note for the asker"/"요청자에게 남길 메모"; "Cancel" | spacer | "Return"/"돌려보내기" (only without chosen option; disabled when empty) | "Accept"/"수락" primary (note optional, sends `option`). Outcomes "Accepted."/"수락했습니다.", "Returned."/"반려했습니다.", "Withdrawn."/"철회했습니다." No "Looks good" label on the card; "Looks good"/"좋습니다" exists only as the push action (PushDelegate.swift:80).

Evidence chips (DecisionCard.swift:296-345): PR (`arrow.triangle.pull`, `owner/repo#N`, opens Safari only for https), commit (sha prefix 8), diff range (`plusminus`), test (green/red/grey by pass), path (last component). `ReviewItem.swift:96-97` has "Asked by %@"/"요청: %@" ("an agent"/"에이전트", "a person"/"사람") but the card header uses `requestedBy.label ?? target.text` (AppModel.swift:111). Verification summary strings "tend ran %lld checks: %lld passed" exist in shared (DecisionModels.swift:476-477).

Decided (history) card (`DecidedCard`, Cards.swift:123-165): outcome chip ("Accepted"/"수락함", "Returned"/"반려함", "Withdrawn"/"철회됨", "Answered"/"답변 완료", "Declined"/"거절됨", "Closed"/"닫음", "Closed by tend"/"tend가 닫음"; green `checkmark` if positive else orange `arrow.uturn.backward`) + time → title (3) → answer/note (3) → host badge · "by %@"/"결정: %@".

### 4. Start work (StartWork.swift)

Sheet from Home toolbar; title "Start Work"/"Work 시작"; toolbar "Cancel"/"취소" and "Start"/"시작" (StartWork.swift:140-155). Form sections:
1. Header "Say what needs to get done. The agents work out the rest." / "해야 할 일만 적으세요. 나머지는 에이전트가 판단합니다."; TextEditor min 140 pt, placeholder "What needs to get done? For example: fix the login timeout on slow networks and add a test." (58-77).
2. "Where"/"위치": Picker "Host"/"호스트" (online and control-mode hosts only, local shown "%@ (dashboard host)"/"%@ (대시보드 호스트)"); Picker "Project"/"프로젝트" from `work-options` projects (last path component; full path in mono below); no-project text "No agent has worked on %@ yet, …" (78-101).
3. "Team"/"팀": Picker "Pipeline"/"파이프라인" + description; off → "Turned off on this dashboard"/"이 대시보드에서 꺼져 있음"; no pipeline → orange warning (102-128).
- Off notice card "Starting work is turned off"/"작업 시작이 꺼져 있습니다" with `allow_work_start` instructions (238-257).
- Start: Face ID/passcode via `LAContext` ("Start agents on %@"), re-validates, POST `/api/device/work` with idempotency key (185-221). Results: "started" dismiss; "unknown" → "It may have started: the host did not answer in time. Check %@ before trying again."; else "%@ did not start it." Failures "Not started: Face ID or the passcode is needed to start work." / "Not started: what this device may do changed. Check and try again."
- No free-form folder, no agent/model choice, no backlog Work start.

### 5. Explore equivalent (Hosts: Screens.swift, Agents.swift)

- iPhone: tab "Hosts"/"호스트" (`server.rack`) → `HostsView` (Agents.swift:61-112): cards of `HostRow` (badge with state dot, alias, summary "%lld agents · %lld working · %lld need you" / "에이전트 %lld개 · %lld개 작업 중 · %lld개가 기다림", orange count capsule for attention). Offline hosts show state label. Order: local first, then alias (Fleet.swift:13-17).
- iPad: sidebar `NavigationSplitView` (Screens.swift:169-210): "Home" (badge) / Section "Hosts" with `HostRow`s / "Settings"; title "Tend"; `.prominentDetail`.
- Host detail (Agents.swift:139-216): header badge 40 + state dot + "This dashboard's host"/"이 대시보드의 호스트" or state label; "tend <ver> · control/observe only" ("제어"/"보기만"); red error when offline. Section "AGENTS n", sorted error > needsInput > asked > limited > working > idle > done; each an `AgentCard`. Empty "No agent runs here."; missing "This host is not in the Fleet snapshot."
- Host states (Theme.swift:244-266): online "Online"/"온라인" green; connecting "Connecting"/"연결 중" blue; degraded "Degraded"/"성능 저하" orange; version_skew "Needs update"/"업데이트 필요" orange; auth_failed "Sign-in failed"/"로그인 실패" red; disabled "Disabled"/"비활성"; else "Offline"/"오프라인" grey.
- No session or pane tree: `paneCount` parsed (Fleet.swift:30, 60) but never rendered; tmux session / pane appear only as rows in agent detail. No shell panes listed, no per-session grouping, no actions on hosts.
- Decide-only: `DecideOnlyNotice` + "Hosts this dashboard covers" list built from the queue with lines "Nothing needs you"/"확인할 항목 없음", "%lld need you", "Could not be read in full: %@" (Agents.swift:67-89, 122-135).

### 6. Agent detail (Agents.swift:220-403) and Work detail (BoardViews.swift:158-273)

Agent detail order: title (`.title2.semibold`: aiTitle > recap > prompt > notification, else "Working"/"No task recorded") → `AgentStatusTag` + `MetaRow` → `NowBox` (label "NEEDS YOU"/"내 차례", "NOW"/"지금", "LAST RESULT"/"마지막 결과" uppercased; tint orange/blue/green 8% fill, 40% stroke; Markdown up to 6000 chars; empty "Nothing recorded yet.") → "Asked"/"맡긴 일" (prompt, 1200 chars) → "Recent output"/"최근 출력" (button "Show recent output"/"최근 출력 보기", then black 86% 280 pt mono box, last 80 lines, polled every 2 s via `/api/device/capture`; only if `canCapture`) → "Where"/"위치" rows Host/Session/Pane/Folder/Model ("호스트/세션/페인/폴더/모델"). Nav title = maker ("claude", "codex"). No actions (no send prompt, stop, answer-in-pane, swap).

Agent status labels (Theme.swift:211-222): "Error"/"오류" red, "Needs input"/"입력 필요" orange, "Asking"/"질문함" orange, "Limited"/"제한됨" purple, "Working"/"작업 중" blue, "Idle"/"대기" mint, "Done"/"완료" green, "Unknown"/"알 수 없음" grey.

Work detail: title → status label (lane `statusLabel`) + repo + time → headline InsetBox (Markdown) → "Stages"/"단계" (StageBar + per-stage dot, name, owner, state "Done"/"완료", "Working"/"작업 중", "Your turn"/"내 차례", "Waiting"/"대기 중") when >1 stage → "Agents"/"에이전트" `AgentRefRow`s (focus agent semibold + tinted stroke) → "Helpers"/"보조 세션" + "Sessions other agents started to help with this work." Gone: "This work is not on the board any more." A single-agent single-stage unit opens the agent detail instead (BoardViews.swift:49-53).

### 7. Settings (Screens.swift:39-96, PushDelegate.swift:155-215)

`Form` sections in order:
1. "Paired with"/"페어링한 곳": "Dashboard"/"대시보드" (origin), "This device"/"이 기기" (name), "Access"/"접근 권한" = "Decide, hosts and new work"/"결정, 호스트, 새 작업" or "Decide only"/"결정만".
2. Unnamed: scope explanation (decideText / operateText incl. 30/90-day expiry) + `HostsAndWorkRequest` upgrade button "Pair again with hosts and new work…" when decide-only.
3. "Notifications"/"알림": toggle "Notify me when something needs me"/"기다리는 일이 생기면 알림 받기"; sub-toggle "Show what was asked"/"물어본 내용 보이기" when host allows content; denied hint; problem line; unsupported texts; footer varies with content switch.
4. Conditional sign-out problem section with destructive "Forget this device here only"/"이 기기에서만 지우기".
5. "Sign out"/"로그아웃" (destructive), confirm "Unpair this device?"/"이 기기의 페어링을 해제할까요?".
No appearance/language/host/account sections; no app version shown.

### 8. Empty and error states

| Situation | UI (file:line) | EN / KO |
|---|---|---|
| Unpaired | `RootView` shows `PairingView` (Screens.swift:114) | "Pair with your dashboard"/"대시보드와 페어링", button "Pair"/"페어링"; scope cards "Decide, see hosts and start work" / "Decide only" (PairingView.swift:72-77) |
| Awaiting approval | PairingView.swift:98-125 | "Approve this phone"/"이 휴대폰 승인하기", 40 pt mono code, "Waiting for approval · the code expires %@" |
| Token refused (401) | `handle` → forgetLocally (AppModel.swift:~690) | "This device is no longer paired. Pair it again."/"이 기기는 더 이상 페어링되어 있지 않습니다. 다시 페어링하세요." |
| Network / server error | red `problem` label atop lane (Home.swift:275-280) | transport/server message verbatim; "Unexpected answer from the dashboard: %@" |
| Fleet read error | In progress / Hosts show `fleetProblem` grey label (Home.swift:353-354, Agents.swift:97) | message |
| Partial read, board | BoardViews.swift:71-94 | "%@: not everything could be read: %@", "%@: showing the first ones; more are waiting.", orange "Not every host's queue could be read, so more may need you.", orange "%lld more need you in a form this app cannot show yet. …" |
| Partial read, other lanes | BoardViews.swift:132-136 | "%lld this app cannot show" |
| Partial history | Home.swift:628-635 | "%@: %@" with "only the latest are shown"/"최근 것만 보여 줍니다" |
| All clear | only if coverage complete (Board.swift:461-464) | TendMark (working+done lit) + "Nothing needs you right now." |
| Loading | `ProgressView` (Home.swift:334, 356, 411) | — |
| Offline host | host row state label, header red error text (Agents.swift:26, 191-193); Start Work hides offline hosts | "Offline"/"오프라인" |
| Decide-only | `DecideOnlyNotice` lock card (Home.swift:500-532) | "This device decides only"/"이 기기는 결정만 할 수 있습니다" + scope-note explanation; 403 → "This device may only decide. Pair it again …" |
| Card action failure | red label under card (DecisionCard.swift:60-65) | server message |
| Push focus miss | outcome (AppModel.swift:1193) | "That item was already decided, or is no longer waiting." |

No dedicated offline/airplane screen, no retry button; recovery is pull-to-refresh (`.refreshable`) and the 10 s poll.

### 9. Notifications (Push.swift, PushDelegate.swift)

- Categories (PushDelegate.swift:76-103): `TEND_OPEN` (no actions), `TEND_WAITING` (no actions), `TEND_REVIEW` [`TEND_ACCEPT` "Looks good"/"좋습니다", `TEND_SEND_BACK` text input "Send back"/"돌려보내기" (destructive, button "Send back", placeholder "What should change?"/"무엇을 바꿔야 하나요?")], `TEND_QUESTION` [`TEND_ANSWER` text input "Answer"/"답하기", button "Send"/"보내기", placeholder "Your answer"/"답"]. All `.authenticationRequired`; none foreground.
- Server format (crates/tend/src/dashboard/push/payload.rs:120-148): title `title-loc-key: PUSH_TITLE` = "Tend: %@ needs you" / "Tend: %@에서 기다리는 일이 있습니다", `thread-id` = host. Without content: `loc-key PUSH_BODY` "%@ new" / "새 항목 %@개", category `TEND_OPEN`. With content: `subtitle` = asker label, `body` = canonical first line (Markdown stripped, <=120 scalars, Push.swift:86-133), category per kind. Custom `tend: {v:1, channel, host, ref}` where ref = HMAC-SHA256 prefix 16 bytes hex (Push.swift:58-65).
- Button press (AppModel.swift:1200-1250): re-reads `/api/device` and `/api/decisions`; `PushActionCheck.verify` requires channel match, content switch on, complete host read, unique ref match, kind match, decidable, body+subtitle unchanged, question without structured options (Push.swift:142-168). Then POST review `accepted`/`returned` (note = typed text) or answer, with `via: notification`. Result shown as local notification title "Tend" body "Accepted."/"Returned."/"Answered."; refused → "Open Tend to decide this."/"Tend를 열어 결정하세요."; unknown → "Tend could not confirm this was sent. Open Tend to check." (PushDelegate.swift:119-150).
- Tap: switches to Home, Your turn, scrolls to and outlines the item with a 2.5 pt orange stroke (`focusedCard`) (Screens.swift:125-129, DecisionCard.swift:67-74).
- Foreground: banner+list+badge+sound, triggers refresh (PushDelegate.swift:107-110). Badge = `actionableCount` set locally too (Screens.swift:233-236).
- Opt-in only: `NotificationOffer` card in Your turn "Get notified when something needs you" with "Turn on"/"켜기" and "Not now"/"나중에" (PushDelegate.swift:218-250); never at first launch.

### 10. Card anatomy per kind

| Kind | Order top→bottom | Source |
|---|---|---|
| Question / review / waiting (`DecisionCard`) | header row [kind label · asker · host badge+alias (remote only) … time or spinner] → verification checks (review) → title (cardTitle, 6 lines) → detail (5) → recommendation → evidence chips → actions or not-here line → failure. Stroke orange 45% (20% if not decidable). | DecisionCard.swift:21-79 |
| Working unit (`WorkCard`, board) | optional "Helper"/"보조" chip + title (2 lines) → StageBar (segments + current stage name + done/total) → headline InsetBox (Your turn 4 lines medium primary; else 2 lines secondary; "Rate limited · the limit policy will resume it") → meta row [status label+icon (lane tint) · agent logos · up to 2 host glyphs+names · "· repo" · helpers count … time]. Your-turn unit stroke orange 45%. Narrow: host names then project drop; large type → two rows. | Cards.swift:365-482 |
| Done unit | same WorkCard; StageBar omitted for single-stage finished; title at 82% opacity; status "Finished"/"완료" `checkmark.circle.fill` green | Cards.swift:377, 381 |
| Agent card (fallback lanes, host detail) | title (2) → status capsule bar 4 pt + maker (not Finished) → headline/status label (2, Finished 3) → `MetaRow` [logo · repo · host badge+alias … time] | Cards.swift:8-36, 91-120 |
| Decided card | outcome chip … time → title → answer/note → host · "by %@" | Cards.swift:123-165 |
| Backlog card | title + "P<n>" chip → line → workspace · pipeline | Cards.swift:486-513 |

Status words on meta row (Cards.swift:253-269): "Planned"/"계획됨" `tray`, "Your turn"/"내 차례" `hand.raised.fill`, "Working"/"작업 중" `circle.dotted.circle`, "Finished"/"완료" `checkmark.circle.fill`. Server keyed texts resolved client-side (Board.swift:39-76): e.g. "Reply from %@ is ready"/"%@의 회신 도착", "Asks you to choose. Open to see the question."/"선택을 기다립니다. 열어서 질문을 확인하세요.", "Stage %@ is Blocked"/"%@ 단계가 막혔습니다", "Waiting on %@: %@", "⟳ Background task"/"⟳ 백그라운드 작업"; unknown keys fall back to focus agent label or "Work".

Time is always relative numeric abbreviated (`.relative(presentation: .numeric, unitsStyle: .abbreviated)`), right-aligned.

### 11. Localization

- All UI strings via `String(localized:)` or `Text("…")`/`Button("…")` LocalizedStringKey literals; `Text(verbatim:)` for data. Catalog `apps/tend-ios/Resources/Localizable.xcstrings`: 291 keys, en + ko, every key has ko. Plus `InfoPlist.xcstrings`.
- `PUSH_TITLE`/`PUSH_BODY` are manual keys (`extractionState: manual`) referenced by the server's `title-loc-key`/`loc-key`.
- Shared Swift (`apps/shared/Sources`, included via project.yml:26) contributes strings (cause texts, "Asked by %@", "tend ran …"), but each app has its own catalog: Mac `apps/tend-macos/Resources/Localizable.xcstrings` has 2369 keys; 135 keys overlap by EN text, translations are separate copies. No sharing with web i18n.
- Notable KO choices (verbatim): "Finished" and "Done" both "완료"; "Return"/"Send back" "돌려보내기" but past outcome "Returned" "반려함"; "Review" "리뷰"; "Needs you" (NowBox) "내 차례"; "Start Work" "Work 시작" while body uses "작업"; "Nothing finished yet." "아직 끝난 Work가 없습니다."; "Asked" "맡긴 일"; "Decide or review" "결정하거나 점검할 것".

### 12. Design: Theme.swift and Mark.swift

Token sourcing: `apps/shared/Sources/TendDesignTokens.swift` is generated by `scripts/design-tokens.mjs` from `apps/shared/Design/tokens.json` (header lines 1-2). Theme reads it for colours via `Color(token:)` (Theme.swift:31-40); type sizes, metrics and some colours are hardcoded.

| Theme symbol | Source | Light / Dark |
|---|---|---|
| TendPalette.orange | token `orange` | #FF8D28 / #FF9230 |
| .blue | `blue` | #0088FF / #0091FF |
| .green | `green` | #34C759 / #30D158 |
| .red | `red` | #FF383C / #FF4245 |
| .purple | `purple` | #CB30E0 / #DB34F2 |
| .mint | `mint` | #00C8B3 / #00DAC3 |
| .accent | `accent` | #007AFF / #007AFF |
| TendSurface.workspace (card fill) | `editor` | #FFFFFF / #1F1F1F |
| .panel (lane, iPhone bg) | `lane` | #F8F8F8 / #252526 |
| .sideBar | `sideBar` | #F8F8F8 / #181818 |
| .border | `border` | #E5E5E5 / #2B2B2B |
| .inputBackground | `inputBackground` | #FFFFFF / #313131 |
| .inputBorder | `inputBorder` | #CECECE / #3C3C3C |
| InsetBox fill / stroke | `inset` / `insetBorder` (Cards.swift:329-330) | #000 3.5% / #FFF 5%; #000 6% / #FFF 7% |
| TendInk.secondary (= metaText) | **hardcoded** (Theme.swift:85) | #5C5C5C / #B0B0B0 (tokens `textSecondary` #000 50% / #FFF 55% not used) |
| TendInk.tertiary | **hardcoded** (88) | #6E6E6E / #8E8E8E |
| TendColor.waiting / error / done | **hardcoded** (114-116) | #FFAA2B / #FF5A3C / #3FBF84 (equal to `Brand` tokens, not read) |
| TendColor.working | **hardcoded** (117) | #16181D / #F2F3F5 (= `Brand.ink`) |
| HostIdentity palette | **hardcoded** (271-274), FNV-1a | 8 pairs #5F63DB/#3E419B … #596FA0/#394A73 (= `hostGradients`, not read) |

Unused tokens on iOS: `cardYourTurn`, `terminal`, `hover`, `pressed`, `segment*`, `text*`, all `Size`, `Metric`, `Mac` roles (grep finds no `TendDesignTokens.Size/Metric/Mac` use).

Radii (hardcoded, Theme.swift:71-73): card 10, lane 12, control 8 (tokens: cardRadius 8, laneRadius 9, controlRadius 4). InsetBox 7 (Cards.swift:329); HostBadge size×0.25; upgrade code box 10, pairing code box 12.

Spacing (hardcoded): card padding 14 (Theme.swift:384; token cardPadding 12); lane item spacing 10 (Home.swift:233; token cardGap 8); board HStack 12 + padding 16; strip padding h16/b10; InsetBox padding 10; card VStack spacing 8 (work), 9 (decision), 7 (decided/backlog); content max width 760. Card button min height 44, h-padding 14 (Cards.swift:226-229). Chip padding h8/v3, 12% tint fill capsule. Stage/status bar 4 pt capsule, stage gap 3.

Type (Dynamic Type styles, Theme.swift:92-109; Mac pt in comments): screenTitle `.title2.semibold`; laneTitle `.headline.bold`; cardTitle `.body.semibold`; body `.subheadline`; detail `.subheadline`; meta `.footnote`; section `.caption.semibold` uppercased, tracking 0.4; mono `.footnote` monospaced. Lane strip count `.title2.bold` monospacedDigit.

TendMark (Mark.swift): F2 four-room mark from `mark-rooms.svg` (viewBox 4 4 56 56), four polygon rooms with one quadratic-rounded outer corner each: waiting = top-right band (y 4–17.25), error = left upper (y 4–35.25), working = right lower (y 22.75–60), done = bottom-left (y 40.75–60) (Mark.swift:31-34). Lit colours: waiting #FFAA2B, error #FF5A3C, working #16181D light / #F2F3F5 dark, done #3FBF84; unlit rooms = working ink at 22% (light) / 26% (dark) opacity (Mark.swift:37-44). Used twice: Home all-clear (waiting off, error off, working+done lit; 44 pt, Home.swift:485) and PairingView header (waiting+working+done lit, PairingView.swift:18). Not used as a per-agent or per-card state mark; agent state uses an 8 pt coloured dot (`AgentStatusTag`, Theme.swift:228-241) and lane tint dots (7 pt).
