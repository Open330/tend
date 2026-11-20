#!/usr/bin/env python3
"""Cut, caption and assemble the demo video from the recorded takes.

    scripts/demo-video/edit.py TAKES_DIR OUT_DIR [--lang en|ko] [--name tend-demo]

TAKES_DIR is the `demo-takes` artifact of .github/workflows/demo-video.yml:
mac-raw.mp4 + marks.txt, {iphone,ipad}-raw.mp4 + -marks.txt. Long stretches
where nothing on screen moves (the driver waiting on the app) are shortened
to a readable hold; nothing is sped up, so every frame shown is a frame the
app drew. Needs ffmpeg and swiftc (macOS: captions are drawn with AppKit).
"""
import argparse, json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FPS = 30
BG = "0xC9CACF"  # the runner's Space Gray desktop, behind the iOS clips too

CAPTIONS = {
    "en": {
        "tagline": "The operations platform for your team of coding agents.",
        "home": "Home: what needs you, what's running, what's done.",
        "question": "Answer an agent's question. Tend delivers it to its terminal.",
        "review": "Review finished work with the agent's evidence, then decide.",
        "explore": "See any pane live, and steer the agent in it.",
        "menubar": "Always one click away in the menu bar.",
        "iphone": "Also on iPhone…",
        "ipad": "…and iPad.",
        "end1": "Tend for Mac, iPhone and iPad",
        "end2": "github.com/Open330/tend",
    },
    "ko": {
        "tagline": "코딩 에이전트 팀을 위한 운영 플랫폼",
        "home": "홈: 내 차례, 진행 중, 끝난 일을 한눈에",
        "question": "에이전트의 질문에 답하면 Tend가 그 터미널로 전달합니다",
        "review": "에이전트가 낸 근거를 보고 끝난 작업을 결정합니다",
        "explore": "어떤 패널이든 실시간으로 보고, 그 자리에서 지시합니다",
        "menubar": "메뉴 막대에서 언제나 한 번에",
        "iphone": "iPhone에서도…",
        "ipad": "…iPad에서도",
        "end1": "Mac, iPhone, iPad용 Tend",
        "end2": "github.com/Open330/tend",
    },
}

# Mac scenes: (first mark, last mark, caption key, hold seconds, crop)
MAC_SCENES = [
    ("home", "question", "home", 1.2, None),
    ("question", "review", "question", 1.8, None),
    ("review", "explore", "review", 2.2, None),
    ("explore", "menubar", "explore", 1.8, None),
    ("menubar", "end", "menubar", 2.0, (1280, 720, 640, 0)),
]


def run(cmd, **kw):
    # No single step of this edit takes minutes; one that does has hung.
    subprocess.run(cmd, check=True, timeout=600, **kw)


def marks(path):
    out = {}
    for line in open(path):
        parts = line.split()
        if len(parts) == 2:
            out.setdefault(parts[0], float(parts[1]))
    return out


def freezes(video, noise="0.00003", min_len="0.8"):
    # Low enough that the cursor alone counts as motion.
    """[(start, end)] where the picture does not change."""
    err = subprocess.run(["ffmpeg", "-hide_banner", "-i", video, "-vf", f"freezedetect=n={noise}:d={min_len}",
                          "-map", "0:v", "-f", "null", "-"], capture_output=True, text=True).stderr
    starts = [float(x) for x in re.findall(r"freeze_start: ([\d.]+)", err)]
    ends = [float(x) for x in re.findall(r"freeze_end: ([\d.]+)", err)]
    duration = float(re.search(r"Duration: (\d+):(\d+):([\d.]+)", err).groups()[2]) + \
        60 * float(re.search(r"Duration: (\d+):(\d+):", err).group(2))
    ends += [duration] * (len(starts) - len(ends))
    return list(zip(starts, ends))


def keep(start, end, frozen, hold):
    """Pieces of [start, end] with each still stretch cut to `hold` seconds."""
    pieces, cursor = [], start
    for f0, f1 in frozen:
        if f1 <= start or f0 >= end:
            continue
        f0, f1 = max(f0, start), min(f1, end)
        if f1 - f0 > hold:
            if f0 + hold > cursor:
                pieces.append((cursor, f0 + hold))
            cursor = f1
    if end > cursor:
        pieces.append((cursor, end))
    return [(a, b) for a, b in pieces if b - a >= 1 / FPS]


def acts(path):
    """[(t, hold)] from the driver's acts.txt."""
    return [(float(t), float(h)) for t, h, *_ in (line.split() for line in open(path)) if t and h]


def keep_acts(start, end, logged, lead=0.15):
    """The acted moments of [start, end] with their holds, merged."""
    spans = sorted((max(start, t - lead), min(end, t + h)) for t, h in logged if start <= t < end)
    merged = []
    for a, b in spans:
        if merged and a <= merged[-1][1] + 0.25:
            merged[-1] = (merged[-1][0], max(merged[-1][1], b))
        elif b - a >= 1 / FPS:
            merged.append((a, b))
    return merged


def cut(video, pieces, out, pre="", post=""):
    """Concatenate `pieces` of `video` (re-timed), through optional filters.
    Each piece is its own seeked input, at a constant frame rate: a
    simulator recording only has frames where the screen changed, and
    trimming its timeline after an fps conversion lands seconds off."""
    inputs = []
    for a, b in pieces:
        inputs += ["-ss", f"{a:.3f}", "-t", f"{b - a:.3f}", "-i", video]
    parts = "".join(f"[{i}:v]fps={FPS},setpts=PTS-STARTPTS[p{i}];" for i in range(len(pieces)))
    concat = "".join(f"[p{i}]" for i in range(len(pieces))) + f"concat=n={len(pieces)}:v=1:a=0[c]"
    chain = f"[c]{pre}fps={FPS},format=yuv420p{post}[v]"
    run(["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex", parts + concat + ";" + chain,
         "-map", "[v]", "-c:v", "libx264", "-preset", "medium", "-crf", "14", out])


def overlay(clip, png, out, start=0.0, end=None, fade=0.25):
    """Lay a full-frame PNG over `clip`, faded in and out."""
    dur = duration(clip)
    end = dur if end is None else end
    alpha = f"fade=t=in:st={start}:d={fade}:alpha=1,fade=t=out:st={max(start, end - fade)}:d={fade}:alpha=1"
    run(["ffmpeg", "-v", "error", "-y", "-i", clip, "-loop", "1", "-t", f"{dur}", "-i", png, "-filter_complex",
         f"[1:v]format=rgba,{alpha}[o];[0:v][o]overlay=0:0:format=auto:shortest=1,format=yuv420p[v]",
         "-map", "[v]", "-t", f"{dur}", "-c:v", "libx264", "-preset", "medium", "-crf", "14", out])


def duration(path):
    return float(subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                                capture_output=True, text=True, check=True).stdout)


def device(video, pieces, out, work, cards, rotate, height, radius):
    """An iOS clip of `pieces`, rounded, on the background. The recording is
    first made constant-frame-rate (simulator recordings only carry a frame
    when the screen changes, and seeking in them lands seconds off); the
    pieces are seconds on that timeline (see `--sheets`)."""
    cfr = cfr_copy(video, work, rotate, height)
    raw = os.path.join(work, os.path.basename(out) + ".raw.mp4")
    cut(cfr, pieces, raw)
    w = int(subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v", "-show_entries", "stream=width",
                            "-of", "csv=p=0", raw], capture_output=True, text=True).stdout)
    mask = os.path.join(work, f"mask-{w}x{height}.png")
    run([cards, "mask", mask, str(w), str(height), str(radius)])
    top = (1080 - 110 - height) // 2 + 6
    # Every input but the clip loops: the output is cut to the clip's length.
    run(["ffmpeg", "-v", "error", "-y", "-i", raw, "-loop", "1", "-t", f"{duration(raw)}", "-i", mask, "-filter_complex",
         f"color=c={BG}:s=1920x1080:r={FPS}[bg];[1:v]format=gray[m];[0:v][m]alphamerge[d];"
         f"[bg][d]overlay=(W-w)/2:{top}:shortest=1,format=yuv420p[v]",
         "-map", "[v]", "-t", f"{duration(raw)}", "-c:v", "libx264", "-preset", "medium", "-crf", "14", out])


def cfr_copy(video, work, rotate, height):
    out = os.path.join(work, os.path.basename(video).replace(".mp4", f"-cfr{height}.mp4"))
    if not os.path.exists(out):
        vf = ("transpose=2," if rotate else "") + f"fps={FPS},scale=-2:{height}:flags=lanczos"
        run(["ffmpeg", "-v", "error", "-y", "-i", video, "-vf", vf, "-c:v", "libx264", "-preset", "fast",
             "-crf", "14", out])
    return out


def xfade_all(clips, out, d=0.4):
    """Join clips with short cross-fades."""
    inputs, chain, offset, last = [], "", 0.0, "0:v"
    for c in clips:
        inputs += ["-i", c]
    for i in range(1, len(clips)):
        offset += duration(clips[i - 1]) - d
        label = f"x{i}"
        chain += f"[{last}][{i}:v]xfade=transition=fade:duration={d}:offset={offset:.3f}[{label}];"
        last = label
    run(["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex", chain + f"[{last}]format=yuv420p[v]",
         "-map", "[v]", "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-profile:v", "high",
         "-movflags", "+faststart", out])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("takes")
    ap.add_argument("out")
    ap.add_argument("--lang", default="en", choices=sorted(CAPTIONS))
    ap.add_argument("--name", default="tend-demo")
    ap.add_argument("--sheets", action="store_true",
                    help="only write contact sheets (2 per second) of the iOS recordings' constant-rate copies")
    args = ap.parse_args()
    if args.sheets:
        work = os.path.join(args.out, "sheets")
        os.makedirs(work, exist_ok=True)
        for slug, rotate, height in (("iphone", False, 940), ("ipad", True, 860)):
            cfr = cfr_copy(os.path.join(args.takes, f"{slug}-raw.mp4"), work, rotate, height)
            run(["ffmpeg", "-v", "error", "-y", "-i", cfr, "-vf", "fps=1,scale=160:-1,tile=20x20",
                 os.path.join(work, f"{slug}-sheet-%d.png")])
            print(slug, round(duration(cfr), 1), "s")
        return
    text = CAPTIONS[args.lang]
    work = os.path.join(args.out, f"work-{args.lang}")
    os.makedirs(work, exist_ok=True)
    cards = os.path.join(args.out, "cards")
    if not os.path.exists(cards):
        run(["swiftc", "-O", os.path.join(HERE, "cards.swift"), "-o", cards])
    logo = os.path.join(REPO, "assets", "logo.svg")

    def caption(key):
        png = os.path.join(work, f"cap-{key}.png")
        run([cards, "caption", png, text[key]])
        return png

    clips = []
    mac = os.path.join(args.takes, "mac-raw.mp4")
    m = marks(os.path.join(args.takes, "marks.txt"))
    # How far the driver's clock runs ahead of the recording (ffmpeg takes
    # a moment to deliver its first frame): TAKES_DIR/mac-offset.txt, in
    # seconds, measured on a visible event such as the popover opening.
    offset_file = os.path.join(args.takes, "mac-offset.txt")
    offset = float(open(offset_file).read()) if os.path.exists(offset_file) else 0.0
    m = {k: v - offset for k, v in m.items()}
    # The driver's log of what it did, when there is one; otherwise still
    # stretches (which on-screen timers can defeat) are shortened.
    act_log = os.path.join(args.takes, "acts.txt")
    logged = [(t - offset, h) for t, h in acts(act_log)] if os.path.exists(act_log) else None
    frozen = None if logged else freezes(mac)
    plan = []
    for first, last, key, hold, crop in MAC_SCENES:
        pieces = keep_acts(m[first], m[last], logged) if logged else keep(m[first], m[last], frozen, hold)
        raw = os.path.join(work, f"mac-{key}.raw.mp4")
        pre = f"crop={crop[0]}:{crop[1]}:{crop[2]}:{crop[3]},scale=1920:1080:flags=lanczos," if crop else ""
        cut(mac, pieces, raw, pre=pre)
        clip = os.path.join(work, f"mac-{key}.mp4")
        overlay(raw, caption(key), clip, start=0.5 if key == "home" else 0.15)
        plan.append({"scene": key, "pieces": [[round(a, 2), round(b, 2)] for a, b in pieces],
                     "seconds": round(duration(clip), 2)})
        clips.append(clip)

    # The name over the first seconds of Home.
    title = os.path.join(work, "title.png")
    run([cards, "title", title, logo, text["tagline"]])
    titled = os.path.join(work, "mac-home-titled.mp4")
    overlay(clips[0], title, titled, start=0.0, end=2.6, fade=0.35)
    clips[0] = titled

    # iOS windows: seconds on each recording's constant-frame-rate copy,
    # picked from its contact sheet (--sheets) into TAKES_DIR/ios-cuts.json.
    cuts = json.load(open(os.path.join(args.takes, "ios-cuts.json")))
    iphone, ipad = [tuple(p) for p in cuts["iphone"]], [tuple(p) for p in cuts["ipad"]]
    for slug, pieces, rotate, height, radius in (("iphone", iphone, False, 940, 64), ("ipad", ipad, True, 860, 28)):
        video = os.path.join(args.takes, f"{slug}-raw.mp4")
        raw = os.path.join(work, f"{slug}.raw.mp4")
        device(video, pieces, raw, work, cards, rotate, height, radius)
        clip = os.path.join(work, f"{slug}.mp4")
        overlay(raw, caption(slug), clip, start=0.15)
        plan.append({"scene": slug, "seconds": round(duration(clip), 2)})
        clips.append(clip)

    end_png = os.path.join(work, "end.png")
    run([cards, "end", end_png, logo, text["end1"], text["end2"]])
    end = os.path.join(work, "end.mp4")
    run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-t", "4", "-i", end_png, "-vf", f"fps={FPS},format=yuv420p",
         "-c:v", "libx264", "-preset", "medium", "-crf", "14", end])
    clips.append(end)

    suffix = "" if args.lang == "en" else f"-{args.lang}"
    final = os.path.join(args.out, f"{args.name}{suffix}.mp4")
    xfade_all(clips, final)
    json.dump(plan, open(os.path.join(args.out, f"plan{suffix}.json"), "w"), indent=1)
    print(final, round(duration(final), 1), "s")
    for p in plan:
        print(p["scene"], p["seconds"])


if __name__ == "__main__":
    sys.exit(main())
