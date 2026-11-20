# Tend demo video — storyboard (WAVE10)

Target: about 80 s, 1920×1080 H.264 MP4 (+ a short WebM/GIF cut for the README).
English captions; a Korean caption variant from the same footage. No voice-over,
no music.

## How it is recorded (nothing on the owner's Mac)

- **macOS (≈80 %)**: a GitHub `macos-26` runner (workflow `demo-video.yml`, on a PR
  labelled `demo-video`). Probed on PR #687: the runner's bash has ScreenCapture
  and Accessibility grants, the virtual display goes to 1920×1080 with
  `displayplacer`, `ffmpeg -f avfoundation` records it at 30 fps with the cursor,
  and `cliclick`/`osascript` drive it. The runner builds Tend.app (Debug) from
  this checkout and the fixture (`docs/demo-setup.sh` on the `tend-demo`
  sandbox, plus the two review requests from `scripts/mac-shots/capture.sh`),
  and the app is started with `TEND_SOCKET` pointing at the sandbox's socket.
  Onboarding, What's New and the notification prompt are pre-dismissed through
  the runner's own defaults. The real app, real window, real menu bar item,
  real cursor; scripted clicks found through the accessibility tree.
- **iPhone and iPad (≈20 %)**: the same runner, after the Mac part: the iOS app
  built for the simulator, paired with the fixture's dashboard (the same fleet
  the Mac shows), driven by an XCUITest while `xcrun simctl io recordVideo` records.
  Headless simulators; shut down and deleted at the end of the job.
- Everything is fixture: project folders `acme-api`, `web-console`, `infra`,
  `tend`; host `local`; agents' prompts from demo-setup. Every frame is OCR'd
  (Vision) and checked for hostnames, `/Users/…`, account names, real repos.

## Scenes

| # | Time | Shows (real app, fixture data) | Caption (EN / KO) |
|---|---|---|---|
| 0 | 0–4 s | Title: Tend icon + name on a plain background. | **Tend** — one place to tend your coding agents / 코딩 에이전트를 한곳에서 돌보는 Tend |
| 1 | 4–14 s | Tend for Mac opens on **Home**: Your turn / In progress / Finished, grouped by project. Cursor drifts across the three lanes. | Home: what needs you, what's running, what's done. / 홈: 내 차례, 진행 중, 끝난 일을 한눈에 |
| 2 | 14–25 s | Click a **Your turn** card that asks a choice ("rotate the JWT signing keys…"). The question and its options open; pick "staging, then prod on approval". The card leaves Your turn. | Answer an agent's question without hunting for its terminal. / 터미널을 찾아다니지 않고 에이전트의 질문에 답하기 |
| 3 | 25–37 s | **Review 5** → the Review sheet: "Merge the proration fix?" with summary, recommendation and the passing test. Choose **merge** → Accept. Count drops to 4. Done. | Decide finished work: summary, evidence, one click. / 끝난 작업 결정: 요약과 근거를 보고 한 번에 |
| 4 | 37–50 s | **Explore**: Needs attention, then the execution topology host › session › window › pane. Click the `sdk` agent (Error) → its live pane. | Explore every session, window and pane — live. / 모든 세션·창·패널을 실시간으로 |
| 5 | 50–57 s | **Steer**: type "retry with the cached spec" into *Send a prompt to this agent/pane* → Send; the text arrives in the live pane. | Steer any agent from the same window. / 같은 창에서 어떤 에이전트든 지시하기 |
| 6 | 57–65 s | Click the **menu bar** item → the popover: agents with states, "6 agents need you · 1 decision". | Always one click away in the menu bar. / 메뉴 막대에서 언제나 한 번에 |
| 7 | 65–72 s | **iPhone** (simulator, framed): Home's lane strip with Your turn; tap a decision, choose. | Also on iPhone… / iPhone에서도… |
| 8 | 72–77 s | **iPad**: the wide layout, three lanes side by side. | …and iPad. / …iPad에서도 |
| 9 | 77–81 s | End card: Tend icon, "macOS · iPhone · iPad", `github.com/Open330/tend`. | — |

Optional if time allows and it reads well: a 3 s Settings → Fleet/Accounts beat
between 6 and 7. Fleet hosts with several machines are skipped: on a runner the
fixture has one host, and a drawn multi-host board would not be the real app.

## Edit

ffmpeg: trim each scene from the take, captions as a lower-third pill (drawtext,
SF/Inter-like system font) over the empty strip under the window (Dock hidden),
0.3 s cross-fades between scenes, iOS clips scaled into device-shaped frames on
the same background. Two outputs: `tend-demo.mp4` (EN) and `tend-demo-ko.mp4`.
README cut: scenes 1–3 as a ≤ 4 MB WebM/GIF.

## Delivery

MP4s are likely 5–15 MB: published as release assets (or a `docs/assets` file if
under a few MB), embedded in README / landing as a link + poster frame. The
workflow and driver scripts are committed so the video can be re-recorded.
