# The demo video

`tend-demo.mp4` (English captions) and `tend-demo-ko.mp4` (Korean), 68 s,
1920×1080, H.264. `poster.png` is the README's still. Storyboard:
[STORYBOARD.md](STORYBOARD.md).

Everything in it is the real Tend for Mac and the real iOS app, and every
piece of data is the demo fixture (`docs/demo-setup.sh`): nothing comes
from anyone's own agents, hosts or accounts.

## Re-recording

1. Label a pull request `demo-video`. `.github/workflows/demo-video.yml`
   builds the app, stands the fixture up on a sandboxed tendd, and records
   on the runner's own screen (`scripts/demo-video/record-mac.sh`: the app
   driven through the accessibility tree with a real cursor) and then in
   iPhone and iPad simulators (`record-ios.sh`, `DemoTourUITests`). The
   takes, a screenshot per step, and the driver's logs are the
   `demo-takes` artifact. Nothing runs on a developer's machine.
2. `scripts/demo-video/edit.py TAKES OUT --sheets` writes contact sheets of
   the simulator recordings; pick the iPhone and iPad windows into
   `TAKES/ios-cuts.json` (seconds on those sheets' timeline). Measure how
   far the Mac take lags the driver's clock on a visible event (the menu
   bar popover opening) into `TAKES/mac-offset.txt`. The ones used for
   this cut are kept here.
3. `edit.py TAKES OUT` and `edit.py TAKES OUT --lang ko`: the Mac scenes
   keep what the driver logged it did (`acts.txt`) and drop the waits;
   captions are drawn with AppKit (`cards.swift`).
4. `scripts/demo-video/privacy-check.sh OUT/tend-demo.mp4 DIR`: Vision OCR
   on two frames a second, checked for paths, host and account names.
   Then watch it.

The fixture panes that answer (`scripts/demo-video/panes/`) read what
Tend sends them and fire the hooks an agent would, so an answer or a
prompt sent from the app visibly lands and the agent goes back to work.
