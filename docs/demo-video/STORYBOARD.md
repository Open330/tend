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

## Scenes (revised after the Codex challenge)

| # | Time | Shows (real app, fixture data) | Caption (EN / KO) |
|---|---|---|---|
| 1 | 0–7 s | Tend for Mac opens on **Home** (Your turn / In progress / Finished, grouped by project) with the Tend name and icon over the first 2 s; the cursor sweeps the lanes once. | Tend: every coding agent, one Home. / Tend: 모든 코딩 에이전트를 한 화면에 |
| 2 | 7–19 s | Click the Your turn card "rotate the JWT signing keys…": Tend reads the agent's picker off its screen and shows the options. Click "staging, then prod on approval": the answer is delivered to the agent's pane, it continues, and the card leaves Your turn. | Answer an agent's question; Tend delivers it to its terminal. / 에이전트의 질문에 답하면 Tend가 그 터미널로 전달합니다 |
| 3 | 19–32 s | **Review** → "Merge the proration fix?": summary, the agent's recommendation and the test it reported. Choose **merge** → Accept; it leaves the decision queue. | Review finished work with the agent's evidence, then decide. / 에이전트가 낸 근거를 보고 끝난 작업을 결정 |
| 4 | 32–50 s | **Explore** → the `sdk` agent stopped on an error (its live pane, its place under host › session › window shown once as navigation). Type "retry with the cached spec and open a PR" → Send; the pane acknowledges and the agent goes back to Working. | See any pane live and steer the agent in it. / 패널을 실시간으로 보고 그 자리에서 에이전트에 지시 |
| 5 | 50–57 s | Click the **menu bar** item → the popover with the agents and what needs you (counts as observed, not scripted). Cropped to the window and the menu cluster; clock and user menu out of frame. | Always one click away in the menu bar. / 메뉴 막대에서 언제나 한 번에 |
| 6 | 57–64 s | **iPhone** (simulator, status bar overridden): the same fleet's Home; accept "Publish the backfilled rollups?". Pairing is out of shot. | Also on iPhone… / iPhone에서도… |
| 7 | 64–70 s | **iPad** landscape: the three lanes side by side. | …and iPad. / …iPad에서도 |
| 8 | 70–74 s | End card: icon, "Tend — macOS · iPhone · iPad", `github.com/Open330/tend`. | — |

Answers and prompts land in fixture panes that behave like an agent
(`scripts/demo-video/panes/`): they read what Tend sends, print it, and fire
the same hooks a real agent fires. Fleet/Accounts are left out (one host on a
runner; a drawn multi-host board is not the real app).

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
