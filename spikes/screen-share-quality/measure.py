#!/usr/bin/env python3
"""Screen-share bench: Mellow's VA-API H.264 screen encoder on reproducible screens.

A 60 Hz source is paced as `crates/mellow-media/src/h264_encoder_linux.rs` paces it (the source's
rate measured with hysteresis, the three layers reopened at 30 or 60 with a keyframe each, token
buckets for the top layer at the encoder's rate and the lower layers at 30, the newest picture
wins, a still screen repeated once a second, and with `--refine` the refining repeats), each
layer encoded by FFmpeg's `h264_vaapi` with Mellow's settings, and measured as it goes on the
network: str0m's packetizer drops the filler data AMD's constant bit rate pads pictures with, so
every byte figure here is without it (the share it had is printed apart). For the `desktop`
scene the top layer is also decoded and compared with the source.

Prints one JSON line. See README.md.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

REPO = Path(__file__).resolve().parents[2]
# Vendored or licence text: files Mellow's own work does not edit, so the scene stays the same.
CODE_SOURCE = REPO / "vendor/str0m/src/packet/h264.rs"
PROSE_SOURCE = REPO / "LICENSE"
MONO = "/usr/share/fonts/TTF/DejaVuSansMono.ttf"
SANS = "/usr/share/fonts/noto/NotoSans-Regular.ttf"
TICK_MS = 1000.0 / 60.0

# Mellow's constants (simulcast.rs, h264_encoder_linux.rs, ADR-0058).
LOWER_RATE = 30
LOW_BITS, QUARTER_BITS = 500_000, 150_000
TOP_BITS = {(1440, 30): 8_000_000, (1440, 60): 8_000_000, (1080, 30): 5_000_000, (1080, 60): 6_000_000,
            (720, 30): 2_500_000, (720, 60): 3_000_000}
RATE_FOR_SIXTY, RATE_FOR_THIRTY = 45, 35
SUSTAINED_UP_MS, SUSTAINED_DOWN_MS = 1000, 2000
REPEAT_MS = 1000
LONGEST_STILL_MS = 300
# What the server counts per packet besides the payload (RTP header, header extensions, SRTP
# tag), and the payload a packet carries at most.
PACKET_OVERHEAD, PACKET_PAYLOAD = 40, 1100
SERVER_BYTES_PER_SECOND = 1_500_000

KEYWORDS = {"fn", "let", "mut", "pub", "use", "struct", "impl", "match", "if", "else", "for", "return",
            "const", "self", "Self", "Some", "None", "Ok", "Err", "while", "in", "crate"}


def render_code(width, height_lines, scale):
    size = round(13 * scale)
    line = round(17 * scale)
    font = ImageFont.truetype(MONO, size)
    lines = CODE_SOURCE.read_text().splitlines()
    gutter = round(300 * scale)
    image = Image.new("RGB", (width, line * height_lines + 40), (30, 30, 46))
    draw = ImageDraw.Draw(image)
    draw.rectangle([0, 0, gutter - 10, image.height], fill=(24, 24, 37))
    for index in range(height_lines):
        text = lines[index % len(lines)][:200]
        y = 10 + index * line
        draw.text((gutter - round(60 * scale), y), f"{index + 1:4}", font=font, fill=(108, 112, 134))
        x = gutter
        if text.lstrip().startswith("//"):
            draw.text((x, y), text, font=font, fill=(127, 132, 156))
            continue
        for word in text.split(" "):
            colour = (203, 166, 247) if word.strip("(&:;,") in KEYWORDS else (205, 214, 244)
            if word.startswith('"'):
                colour = (166, 227, 161)
            draw.text((x, y), word, font=font, fill=colour)
            x += font.getlength(word + " ")
        if index < 70:
            draw.text((round(12 * scale), y), f"src/module_{index:02}.rs", font=font, fill=(166, 173, 200))
    return np.asarray(image)


def render_document(width, height, scale, typed=""):
    font = ImageFont.truetype(SANS, round(15 * scale))
    line = round(22 * scale)
    image = Image.new("RGB", (width, height), (250, 250, 250))
    draw = ImageDraw.Draw(image)
    margin = round(220 * scale)
    wrapped = []
    for paragraph in PROSE_SOURCE.read_text().splitlines():
        current = ""
        for word in paragraph.split():
            candidate = (current + " " + word).strip()
            if font.getlength(candidate) > width - 2 * margin:
                wrapped.append(current)
                current = word
            else:
                current = candidate
        wrapped.append(current)
    y = 20
    for text in wrapped:
        if y > height - 2 * line:
            break
        draw.text((margin, y), text, font=font, fill=(30, 30, 30))
        y += line
    draw.rectangle([margin - 8, height - 2 * line - 6, width - margin, height - 6], fill=(235, 235, 245))
    draw.text((margin, height - 2 * line), typed, font=font, fill=(20, 20, 20))
    return np.asarray(image)


def moving_region(width, height, phase, fine=False):
    yy, xx = np.mgrid[0:height, 0:width]
    k = 3 if fine else 1
    red = 127 + 120 * np.sin(xx / (23 / k) + phase)
    green = 127 + 120 * np.sin(yy / (17 / k) - phase * 1.3)
    blue = 127 + 120 * np.sin((xx + yy) / (31 / k) + phase * 0.7)
    region = np.stack([red, green, blue], axis=-1).astype(np.uint8)
    cx = int((width / 2) + (width / 3) * np.sin(phase))
    cy = int((height / 2) + (height / 3) * np.cos(phase * 0.8))
    region[max(cy - 40, 0):cy + 40, max(cx - 40, 0):cx + 40] = (250, 250, 250)
    return region


class Scene:
    """`desktop`: code, scroll, still, document, typing, video region (12 s). `switch`: a video
    region over code changing 60 times a second, then 10 (10 s). `fifteen`: 15 a second (6 s).
    `heavy`: the whole screen moving finely 60 times a second (6 s). `heavy-switch`: the whole
    screen changing 60 times a second, then 10 (10 s)."""

    PHASES = {
        "desktop": [(0, 2, "still code"), (2, 4, "scroll 60/s"), (4, 6, "still after scroll"),
                    (6, 8, "still document"), (8, 10, "typing 10/s"), (10, 12, "video region 60/s")],
        "switch": [(0, 5, "region 60/s"), (5, 10, "region 10/s")],
        "fifteen": [(0, 6, "region 15/s")],
        "heavy": [(0, 6, "whole screen 60/s")],
        "heavy-switch": [(0, 5, "whole screen 60/s"), (5, 10, "whole screen 10/s")],
    }

    def __init__(self, name, width, height):
        self.name, self.width, self.height = name, width, height
        self.phases = self.PHASES[name]
        self.seconds = self.phases[-1][1]
        self.ticks = self.seconds * 60
        self.scale = height / 1440
        lines = int(height / (17 * self.scale)) * 3
        self.code = render_code(width, lines, self.scale)
        self.document = render_document(width, height, self.scale) if name == "desktop" else None
        self.typing_text = "The quick brown fox jumps over the lazy dog while the encoder keeps every glyph sharp. " * 2
        self.cache = {}

    def scroll_offset(self, tick):
        if self.name != "desktop" or tick < 120:
            return 0
        return round((min(tick, 240) - 120) * 4 * self.scale)

    def region_rate(self, tick):
        """How many times a second the moving region changes at `tick`."""
        second = tick / 60
        if self.name in ("switch", "heavy-switch"):
            return 60 if second < 5 else 10
        if self.name == "fifteen":
            return 15
        return 60

    def key(self, tick):
        second = tick / 60
        if self.name == "desktop":
            if second < 6:
                return ("code", self.scroll_offset(tick))
            if second < 8:
                return ("doc", 0)
            if second < 10:
                return ("doc", (tick - 480) // 6 + 1)
            return ("region", tick)
        step = 60 // self.region_rate(tick)
        kind = "heavy" if self.name.startswith("heavy") else "region"
        return (kind, tick - tick % step)

    def frame(self, tick):
        key = self.key(tick)
        if key in self.cache:
            return self.cache[key]
        kind, value = key
        if kind == "code":
            picture = self.code[value:value + self.height]
        elif kind == "doc":
            picture = self.document if value == 0 else render_document(
                self.width, self.height, self.scale, self.typing_text[:value])
        elif kind == "heavy":
            picture = moving_region(self.width, self.height, value * 0.15, fine=True)
        else:
            picture = self.code[self.scroll_offset(value):][: self.height].copy()
            w, h = round(640 * self.scale), round(360 * self.scale)
            x0, y0 = self.width - w - 40, 60
            picture[y0:y0 + h, x0:x0 + w] = moving_region(w, h, value * 0.15)
        picture = np.ascontiguousarray(picture)
        if len(self.cache) > 64:
            self.cache.clear()
        self.cache[key] = picture
        return picture

    def changed(self, tick):
        return tick == 0 or self.key(tick) != self.key(tick - 1)

    def text_crop(self):
        h, w = self.height, self.width
        return (slice(h // 4, h // 4 + h // 3), slice(w // 4, w // 4 + w // 3))


def luma(rgb):
    # BT.709, limited range, as VA-VPP writes NV12 for the encoder.
    r, g, b = (rgb[..., index].astype(np.float32) for index in range(3))
    return np.clip(np.rint(16 + 219 / 255 * (0.2126 * r + 0.7152 * g + 0.0722 * b)), 0, 255).astype(np.uint8)


# --- Mellow's pacing, at 1 ms steps --------------------------------------------------------------

class Bucket:
    def __init__(self, rate):
        self.rate, self.tokens, self.at = float(rate), 1.0, None

    def tokens_at(self, now):
        earned = 0.0 if self.at is None else (now - self.at) / 1000 * self.rate
        return min(self.tokens + earned, 1.5)

    def has(self, now):
        return self.tokens_at(now) >= 1 - 1e-9

    def is_full(self, now):
        return self.tokens_at(now) >= 1.5 - 1e-9

    def take(self, now):
        self.tokens = max(self.tokens_at(now) - 1.0, -1.0)
        self.at = now


class SourceRate:
    def __init__(self):
        self.pictures, self.rate, self.since = [], 30, None

    def picture(self, now):
        self.pictures = [at for at in self.pictures[-119:] if now - at < 1000] + [now]
        count = len(self.pictures)
        if self.rate == 30:
            asks, sustained, other = count >= RATE_FOR_SIXTY, SUSTAINED_UP_MS, 60
        else:
            asks, sustained, other = count <= RATE_FOR_THIRTY, SUSTAINED_DOWN_MS, 30
        if not asks:
            self.since = None
            return self.rate
        if self.since is None:
            self.since = now
        if now - self.since >= sustained:
            self.rate, self.since = other, None
        return self.rate


def schedule(scene, rate, refine):
    """Encodes as `EncodingLoop` and `Pacing` plan them: a list of (time ms, layer, segment,
    source tick, kind) with kind "new", "repeat" or "refine" for the top layer ("lower" below);
    a segment is one opening of the encoders. `rate` is 30, 60 or "auto"; `refine` is
    (shortest still ms, window ms) or None for the pacing without refining."""
    source = SourceRate()
    encoder_rate = 30 if rate == "auto" else rate
    segment = 0
    top, lower = Bucket(encoder_rate), Bucket(LOWER_RATE)
    last_top = last_change = last_picture = interval = waiting = current = None
    is_lower_pending = False
    encodes, segments = [], [encoder_rate]
    arrivals = {int(np.ceil(tick * TICK_MS)): tick for tick in range(scene.ticks) if scene.changed(tick)}
    for now in range(scene.seconds * 1000):
        if now in arrivals:
            measured = source.picture(now)
            if rate == "auto" and measured != encoder_rate:
                encoder_rate = measured
                segment += 1
                segments.append(encoder_rate)
                top, last_top = Bucket(encoder_rate), None
            waiting = arrivals[now]
        takes = waiting is not None and top.has(now)
        repeat_due = last_top is not None and now - last_top >= REPEAT_MS
        still = refine[0] if refine else 0
        if refine and interval is not None and interval < refine[1]:
            # Twice the source's last interval, within the bounds (`still_before_refining`).
            still = min(max(2 * interval, refine[0]), LONGEST_STILL_MS)
        refine_due = (refine is not None and last_change is not None
                      and still <= now - last_change < refine[1] and top.is_full(now))
        if takes or repeat_due or refine_due:
            if takes:
                current, waiting, kind, last_change = waiting, None, "new", now
                interval = None if last_picture is None else now - last_picture
                last_picture = now
            else:
                kind = "repeat" if repeat_due else "refine"
            encodes.append((now, "h", segment, current, kind))
            top.take(now)
            last_top, is_lower_pending = now, True
        if is_lower_pending and lower.has(now):
            lower.take(now)
            is_lower_pending = False
            for layer in ("l", "q"):
                encodes.append((now, layer, segment, current, "lower"))
    return encodes, segments


# --- encoding ------------------------------------------------------------------------------------

def layer_size(width, height, layer):
    divisor = {"h": 1, "l": 2, "q": 4}[layer]
    return max((width // divisor) & ~1, 2), max((height // divisor) & ~1, 2)


def strip_filler(data):
    """`data` without filler-data NAL units (type 12), as str0m's packetizer sends it."""
    starts = [m.start() for m in re.finditer(b"\x00\x00\x01", data)]
    kept = bytearray()
    for begin, end in zip(starts, starts[1:] + [len(data)]):
        unit_end = end - 1 if end < len(data) and data[end - 1] == 0 else end
        unit = data[begin:unit_end]
        if unit[3] & 31 != 12:
            kept += b"\x00" + unit
    return bytes(kept)


def packet_sizes(path):
    result = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "packet=size", "-of", "csv=p=0", str(path)],
                            capture_output=True, text=True, check=True)
    return [int(line.split(",")[0]) for line in result.stdout.split()]


def encode(scene, frames, layer, rate, bits, args, out):
    width, height = layer_size(scene.width, scene.height, layer)
    size = "" if layer == "h" else f"w={width}:h={height}:"
    command = [
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-vaapi_device", "/dev/dri/renderD128",
        "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{scene.width}x{scene.height}", "-r", str(rate), "-i", "-",
        "-vf", f"format=bgr0,hwupload,scale_vaapi={size}format=nv12:out_color_matrix=bt709:out_range=tv",
        "-c:v", "h264_vaapi", "-profile:v", args.profile, "-bf", "0", "-g", "3000", "-rc_mode", "CBR",
        "-async_depth", "1", "-b:v", str(bits), "-maxrate", str(bits), "-bufsize", str(bits // 2),
        "-compression_level", str(args.level),
    ]
    if args.coder:
        command += ["-coder", args.coder]
    command += ["-f", "h264", str(out)]
    process = subprocess.Popen(command, stdin=subprocess.PIPE)
    for tick in frames:
        process.stdin.write(scene.frame(tick).tobytes())
    process.stdin.close()
    if process.wait() != 0:
        sys.exit("encode failed")
    padded = out.read_bytes()
    stripped = strip_filler(padded)
    network = out.with_suffix(".network.h264")
    network.write_bytes(stripped)
    out.unlink()
    sizes = packet_sizes(network)
    assert len(sizes) == len(frames), (layer, len(sizes), len(frames))
    return network, sizes, len(padded) - len(stripped)


# --- metrics -------------------------------------------------------------------------------------

def block_ssim(a, b):
    """FFmpeg's SSIM: 8x8 windows every 4 pixels, Y plane."""
    a = a.astype(np.float64)
    b = b.astype(np.float64)

    def window_sums(x):
        s = np.pad(x, ((1, 0), (1, 0))).cumsum(0).cumsum(1)
        return s[8::4, 8::4] - s[:-8:4, 8::4] - s[8::4, :-8:4] + s[:-8:4, :-8:4]

    n = 64.0
    sa, sb = window_sums(a), window_sums(b)
    saa, sbb, sab = window_sums(a * a), window_sums(b * b), window_sums(a * b)
    c1, c2 = (0.01 * 255) ** 2 * n * n, (0.03 * 255) ** 2 * n * n * (n - 1) / n
    fs1, fs12 = sa * sa + sb * sb, sa * sb
    ssim = ((2 * fs12 + c1) * (2 * (sab * n - fs12) + c2)) / ((fs1 + c1) * ((saa + sbb) * n - fs1 + c2))
    return float(ssim.mean())


def psnr(a, b):
    mse = np.mean((a.astype(np.int32) - b.astype(np.int32)) ** 2)
    return 99.0 if mse == 0 else float(10 * np.log10(255 * 255 / mse))


def decoded_luma(paths, width, height):
    """The Y plane (limited range, as decoded) of each picture of the streams, in order."""
    size = width * height
    for path in paths:
        process = subprocess.Popen(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(path),
                                    "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"], stdout=subprocess.PIPE)
        while True:
            chunk = process.stdout.read(size * 3 // 2)
            if len(chunk) < size * 3 // 2:
                break
            yield np.frombuffer(chunk[:size], np.uint8).reshape(height, width)
        process.wait()


def on_the_wire(size):
    return size + PACKET_OVERHEAD * -(-size // PACKET_PAYLOAD)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", required=True, type=Path,
                        help="where the encoded streams go (never the repository)")
    parser.add_argument("--scene", default="desktop", choices=sorted(Scene.PHASES))
    parser.add_argument("--size", default="2560x1440")
    parser.add_argument("--rate", default="auto", help="encoder rate: 30, 60 or auto (ADR-0058)")
    parser.add_argument("--level", type=int, default=21, help="compression_level (radeonsi quality bits)")
    parser.add_argument("--profile", default="constrained_baseline")
    parser.add_argument("--coder")
    parser.add_argument("--refine", help="still ms,window ms of the refining repeats")
    parser.add_argument("--quality", action="store_true", help="decode and compare (desktop scene)")
    parser.add_argument("--every", type=int, default=2, help="compare every Nth tick")
    parser.add_argument("--name", required=True)
    args = parser.parse_args()
    out_dir = args.out_dir.resolve()
    if out_dir == REPO or REPO in out_dir.parents:
        sys.exit("--out-dir must be outside the repository")
    out_dir.mkdir(parents=True, exist_ok=True)
    width, height = map(int, args.size.split("x"))
    scene = Scene(args.scene, width, height)
    rate = "auto" if args.rate == "auto" else int(args.rate)
    refine = tuple(map(int, args.refine.split(","))) if args.refine else None
    encodes, segments = schedule(scene, rate, refine)

    # Encode each layer of each opening of the encoders.
    sizes = [0] * len(encodes)
    filler = 0
    top_streams = []
    for segment, segment_rate in enumerate(segments):
        for layer in ("h", "l", "q"):
            indices = [i for i, e in enumerate(encodes) if e[1] == layer and e[2] == segment]
            if not indices:
                continue
            bits = TOP_BITS[(height, segment_rate)] if layer == "h" else (LOW_BITS if layer == "l" else QUARTER_BITS)
            layer_rate = segment_rate if layer == "h" else LOWER_RATE
            path = out_dir / f"{args.name}.{segment}.{layer}.h264"
            network, layer_sizes, removed = encode(scene, [encodes[i][3] for i in indices], layer, layer_rate,
                                                   bits, args, path)
            filler += removed
            for i, size in zip(indices, layer_sizes):
                sizes[i] = size
            if layer == "h":
                top_streams.append(network)

    times = np.array([e[0] for e in encodes])
    wire = np.array([on_the_wire(s) for s in sizes])
    is_top = np.array([e[1] == "h" for e in encodes])

    def worst_second(start_ms, end_ms):
        """The heaviest one-second window lying within [start_ms, end_ms)."""
        starts = range(start_ms, max(end_ms - 1000, start_ms) + 1)
        return max(int(wire[(times >= t) & (times < t + 1000)].sum()) for t in starts)

    arrivals = {tick: int(np.ceil(tick * TICK_MS)) for tick in range(scene.ticks)}
    phases = {}
    for start, end, name in scene.phases:
        lo, hi = start * 1000, end * 1000
        in_phase = (times >= lo) & (times < hi)
        top_in_phase = [e for e in encodes if e[1] == "h" and lo <= e[0] < hi]
        new = [e for e in top_in_phase if e[4] == "new"]
        delays = [at - arrivals[tick] for at, _, _, tick, _ in new]
        shown = np.diff([at for at, *_ in new])
        expected = np.diff([arrivals[tick] for _, _, _, tick, _ in new])
        phases[name] = {
            "network_kbit_s": round(int(wire[in_phase].sum()) * 8 / (end - start) / 1000),
            "top_kbit_s": round(int(wire[in_phase & is_top].sum()) * 8 / (end - start) / 1000),
            "worst_second_kB": round(worst_second(lo, hi) / 1000, 1),
            "top_encodes_per_s": round(len(top_in_phase) / (end - start), 1),
            "new_per_s": round(len(new) / (end - start), 1),
            "delay_ms_mean_max": [round(float(np.mean(delays)), 1) if delays else 0, int(max(delays, default=0))],
            "jitter_ms": round(float(np.std(shown - expected)), 1) if len(shown) else 0.0,
        }
    summary = {
        "name": args.name, "scene": args.scene, "size": args.size, "rate": args.rate, "refine": args.refine,
        "segments": segments,
        "reopened_at_ms": [min(e[0] for e in encodes if e[2] == segment) for segment in range(1, len(segments))],
        "network_kbit_s": round(int(wire.sum()) * 8 / scene.seconds / 1000),
        "worst_second_kB": round(worst_second(0, scene.seconds * 1000) / 1000, 1),
        "server_limit_kB": SERVER_BYTES_PER_SECOND / 1000,
        "filler_dropped_by_str0m_kbit_s": round(filler * 8 / scene.seconds / 1000),
        "phases": phases,
    }

    if args.quality:
        crop = scene.text_crop()
        tops = [e for e in encodes if e[1] == "h"]
        decoded = decoded_luma(top_streams, width, height)
        references, rows = {}, {name: [] for _, _, name in scene.phases}
        shown = shown_tick = None
        next_top = 0
        for tick in range(scene.ticks):
            while next_top < len(tops) and tops[next_top][0] <= tick * TICK_MS:
                shown, shown_tick = next(decoded), tops[next_top][3]
                next_top += 1
            if tick % args.every or shown is None:
                continue
            key = scene.key(shown_tick)
            if key not in references:
                if len(references) > 64:
                    references.clear()
                references[key] = luma(scene.frame(shown_tick))
            own = references[key]
            phase = next(name for start, end, name in scene.phases if start * 60 <= tick < end * 60)
            rows[phase].append((psnr(shown, own), block_ssim(shown, own),
                                psnr(shown[crop], own[crop]), block_ssim(shown[crop], own[crop])))
        for name, values in rows.items():
            phases[name]["psnr_ssim_text_psnr_ssim"] = [
                round(float(np.mean([row[i] for row in values])), 4 if i % 2 else 2) for i in range(4)]
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
