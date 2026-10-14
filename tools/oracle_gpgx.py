#!/usr/bin/env python3
"""Drive Genesis Plus GX as a plain shared library and ask it one question.

The whole point is independence. This emulator shares no code, no decoder and
no assumptions with ours, so if it agrees that the cartridge lands on the fault
screen, the cartridge is at fault. If it boots the game, our core is wrong.

Loadable through ctypes rather than a frontend: there is no SDL here, and no
window is needed to answer the question.
"""
import ctypes, sys, zlib, pathlib, json, os
from libretro_abi import GameInfo, AvInfo, Variable, AudioSample, AudioSampleBatch

def _find_core():
    """Locate the libretro core.

    The core is a build artifact, so it lives under cache/ and is not next to
    this script. Resolving it relative to __file__ alone worked only while
    the two happened to sit together, and broke the moment the script moved.
    """
    here = pathlib.Path(__file__).resolve().parent
    candidates = [
        here / "genesis_plus_gx_libretro.so",
        here.parent / "cache" / "oracle" / "genesis_plus_gx_libretro.so",
        here.parent / "cache" / "oracle" / "Genesis-Plus-GX" / "genesis_plus_gx_libretro.so",
    ]
    if os.environ.get("MDX_ORACLE_CORE"):
        candidates.insert(0, pathlib.Path(os.environ["MDX_ORACLE_CORE"]))
    for c in candidates:
        if c.exists():
            return c
    raise SystemExit(
        "genesis_plus_gx_libretro.so not found; build it with "
        "make -f libretro.mk in cache/oracle/Genesis-Plus-GX"
    )


LIB = ctypes.CDLL(str(_find_core()))

# --- libretro constants ----------------------------------------------------
RETRO_ENVIRONMENT_GET_LOG_INTERFACE   = 27
RETRO_ENVIRONMENT_SET_PIXEL_FORMAT    = 10
RETRO_ENVIRONMENT_GET_VARIABLE        = 15
RETRO_ENVIRONMENT_SET_VARIABLES       = 16
RETRO_ENVIRONMENT_SET_MEMORY_MAPS     = (36 | 0x10000)
RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS = 11
RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL = 8

frames = []          # (w, h, pitch, bytes)
samples = []
last = None
pixel_format = [0]
variables = {}
variable_buffers = {}

# --- libretro callbacks ----------------------------------------------------
ENV = ctypes.CFUNCTYPE(ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p)
VID = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint,
                       ctypes.c_uint, ctypes.c_size_t)
SNAPSHOT = ctypes.CFUNCTYPE(None)
AUD = AudioSampleBatch
AUD_SINGLE = AudioSample
INPUT_POLL = ctypes.CFUNCTYPE(None)
INPUT = ctypes.CFUNCTYPE(ctypes.c_int16, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint)
TIME = ctypes.CFUNCTYPE(ctypes.c_uint64)

def env_cb(cmd, data):
    if cmd == RETRO_ENVIRONMENT_SET_PIXEL_FORMAT:
        pixel_format[0] = ctypes.cast(data, ctypes.POINTER(ctypes.c_uint)).contents.value
        return 1
    if cmd == RETRO_ENVIRONMENT_GET_VARIABLE:
        v = ctypes.cast(data, ctypes.c_void_p).value
        if v is None:
            return 0
        sv = ctypes.cast(data, ctypes.POINTER(Variable)).contents
        found = variables.get(sv.key.decode(), None) if sv.key else None
        if found is None:
            sv.value = None
            return 0
        out = ctypes.create_string_buffer(found.encode())
        variable_buffers[sv.key] = out
        sv.value = ctypes.cast(out, ctypes.c_char_p)
        return 1
    # Everything else (directories, variable registration, performance hints)
    # can be declined safely: the core defaults are what we want.
    return 0

def vid_cb(data, width, height, pitch):
    if not data:
        return
    raw = ctypes.string_at(data, pitch * height)
    frames.append((width, height, pitch, raw))

def snap_cb(): frames.append(None)
audio_capture = None
audio_error = None
def aud_cb(data, size):
    global audio_error
    if audio_capture is not None and data:
        try: audio_capture.batch(data, size)
        except Exception as error: audio_error = error
    return size

def aud_single_cb(left, right):
    global audio_error
    if audio_capture is not None:
        try: audio_capture.single(left, right)
        except Exception as error: audio_error = error
pressed_buttons = [set(), set(), set(), set()]
LIBRETRO_BUTTONS = {0: "b", 1: "a", 3: "start", 4: "up", 5: "down", 6: "left", 7: "right", 8: "c"}


def input_cb(port, device, index, button):
    if port >= 4 or device & 0xff != 1 or index != 0:
        return 0
    if button == 256:  # RETRO_DEVICE_ID_JOYPAD_MASK
        return sum(1 << k for k, v in LIBRETRO_BUTTONS.items() if v in pressed_buttons[port])
    return int(LIBRETRO_BUTTONS.get(button) in pressed_buttons[port])


def poll_cb(): pass
def time_cb(): return 0

env = ENV(env_cb); vid = VID(vid_cb); snap = SNAPSHOT(snap_cb)
aud = AUD(aud_cb); inp = INPUT(input_cb); poll = INPUT_POLL(poll_cb); tick = TIME(time_cb)
aud_single = AUD_SINGLE(aud_single_cb)

def run(rom_path, frames_wanted, input_events=None, capture=None):
    global audio_capture, audio_error
    audio_capture, audio_error = capture, None
    input_events = input_events or []
    for port in pressed_buttons:
        port.clear()
    next_event = 0
    LIB.retro_set_environment.restype = ctypes.c_void_p
    LIB.retro_set_environment(ctypes.cast(env, ctypes.c_void_p))
    LIB.retro_set_video_refresh(ctypes.cast(vid, ctypes.c_void_p))
    LIB.retro_set_audio_sample(ctypes.cast(aud_single, ctypes.c_void_p))
    LIB.retro_set_audio_sample_batch(ctypes.cast(aud, ctypes.c_void_p))
    LIB.retro_set_input_poll(ctypes.cast(poll, ctypes.c_void_p))
    LIB.retro_set_input_state(ctypes.cast(inp, ctypes.c_void_p))
    LIB.retro_init()

    av = AvInfo()
    LIB.retro_get_system_av_info(ctypes.byref(av))
    info = GameInfo(str(rom_path).encode(), None, 0, None)
    ok = LIB.retro_load_game(ctypes.byref(info))
    if not ok:
        raise RuntimeError('reference core refused cartridge')
    if capture is not None:
        LIB.retro_get_system_av_info(ctypes.byref(av))
        capture.configure(av.timing.sample_rate)
    print(json.dumps({"load_game_returned": ok,
                      "av_base": [av.geometry.base_width, av.geometry.base_height,
                                  av.geometry.max_width, av.geometry.max_height]}))
    observer_frame=getattr(LIB,"retro_mdx_trace_set_frame",None)
    if os.environ.get("MDX_ORACLE_TRACE_TIMING") and observer_frame is None:
        raise RuntimeError("timing sidecar requires an oracle exporting retro_mdx_trace_set_frame")
    if observer_frame is not None:
        observer_frame.argtypes=[ctypes.c_uint];observer_frame.restype=None
    for n in range(1, frames_wanted + 1):
        if capture is not None: capture.begin_frame(n - 1)
        if observer_frame is not None:observer_frame(n-1)
        while next_event < len(input_events) and input_events[next_event]["frame"] <= n - 1:
            event = input_events[next_event]
            pressed_buttons[event.get("port", 0)] = set(event["buttons"])
            next_event += 1
        frames.clear()
        LIB.retro_run()
        if n in (1, 10, 30, 60, 120, 180, 240, 360, 480, 600) or n == frames_wanted:
            if frames and frames[-1] is not None:
                w, h, pitch, raw = frames[-1]
                d = describe(w, h, pitch, raw, pixel_format[0])
                samples.append({"frame": n, "w": w, "h": h, **d})
                if n == frames_wanted:
                    last = frames[-1]
    if audio_error is not None:
        raise RuntimeError('reference audio capture failed') from audio_error
    return last

class CpuCoreInfo(ctypes.Structure):
    _fields_ = [("key", ctypes.c_char_p),
                ("ctx", ctypes.c_void_p),
                ("type", ctypes.c_int),
                ("cache", ctypes.c_bool)]


def save_state():
    """Genesis Plus GX's whole machine state, VDP register file included.

    retro_serialize is the only API that exposes the oracle's VDP registers:
    retro_get_memory_data covers RAM and VRAM but not the chip's own state,
    and the register file changes the instant a control-port write happens,
    so it cannot be inferred from memory a frame later.
    """
    LIB.retro_serialize_size.restype = ctypes.c_size_t
    n = LIB.retro_serialize_size()
    if not n:
        return None
    LIB.retro_serialize.restype = ctypes.c_size_t
    LIB.retro_serialize.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    buf = ctypes.create_string_buffer(n)
    if not LIB.retro_serialize(buf, n):
        return None
    return bytes(buf)


# Our core's VDP registers from frame 158 onwards, as a search key. The order
# is r0..r31 so a contiguous hit locates the oracle's register file exactly.
OUR_VDP_REGS = bytes([0x04, 0x34, 0x28, 0x30, 0x05, 0x6E, 0, 0, 0, 0,
                      0x01, 0x03, 0x81, 0x08, 0, 0x02, 0x03] + [0] * 15)


def find_vdp_regs_in_state(blob):
    if not blob:
        return None, None
    for n in (17, 32):
        for width in (1, 2):
            if width == 1:
                key = OUR_VDP_REGS[:n]
            else:
                key = b"".join(bytes((OUR_VDP_REGS[i], 0)) for i in range(n))
            i = blob.find(key)
            if i >= 0:
                step = width
                return i, [blob[i + k * step] if width == 1
                           else blob[i + k * 2] for k in range(32)]
    return None, None


def read_68k_registers():
    """The 68k register file straight out of Genesis Plus GX.

    retro_get_cpu_core_info hands back the m68k core's REG_D / REG_A / PC,
    which is the only way to ask the oracle what a register held at a given
    frame. Reading a register out of work RAM would only show what the game
    stored, and the loop under investigation never stores d0 anywhere.
    """
    if not hasattr(LIB, "retro_get_cpu_core_info"):
        return None
    LIB.retro_get_cpu_core_info.restype = ctypes.c_size_t
    LIB.retro_get_cpu_core_info.argtypes = [ctypes.c_size_t,
                                            ctypes.POINTER(CpuCoreInfo)]
    info = CpuCoreInfo()
    if not LIB.retro_get_cpu_core_info(0, ctypes.byref(info)):
        return None
    names = ["z80", "m68k"]
    if info.type >= len(names):
        return None
    if info.type == 1:
        d = ctypes.cast(info.ctx, ctypes.POINTER(ctypes.c_uint32))
        a = ctypes.cast(info.ctx, ctypes.POINTER(ctypes.c_uint32))
        # GPGX's m68k core reports REG_D first, then REG_A, then PC, then SR.
        base = ctypes.cast(info.ctx, ctypes.POINTER(ctypes.c_uint32))
        def r(i):
            return base[i]
        return {"core": "m68k", "d": [r(i) for i in range(8)],
                "a": [r(8 + i) for i in range(8)],
                "pc": r(16), "sr": r(17)}
    return {"core": names[info.type]}


def read_work_ram():
    """Return 64 KiB in logical 68000 byte order, not raw host-word order.

    The pinned GPGX core uses native uint16 storage and READ/WRITE_BYTE XOR 1
    on little-endian hosts. This exporter normalizes that representation only.
    """
    from md_bytes import word_bytes
    import sys
    RETRO_MEMORY_SYSTEM_RAM = 2
    LIB.retro_get_memory_data.restype = ctypes.c_void_p
    LIB.retro_get_memory_data.argtypes = [ctypes.c_uint]
    p = LIB.retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)
    return word_bytes(ctypes.string_at(p, 0x10000), sys.byteorder == "little") if p else None

def write_png(path, w, h, pitch, raw, px_fmt):
    """Minimal PNG writer, so no image library is needed to eyeball a frame."""
    if px_fmt == 1:   # little-endian XRGB8888: bytes B,G,R,X
        rows = []
        for y in range(h):
            off = y * pitch
            rows.append(b"".join(
                bytes((raw[off + i + 2], raw[off + i + 1], raw[off + i]))
                for i in range(0, w * 4, 4)))
    else:                # pixel format 2 = RGB565; 0 = 0RGB1555
        rows = []
        for y in range(h):
            off = y * pitch
            row = bytearray()
            for i in range(0, w * 2, 2):
                v = raw[off + i] | (raw[off + i + 1] << 8)
                row += bytes((((v >> (11 if px_fmt == 2 else 10)) & 31) * 255 // 31,
                              ((v >> 5) & (63 if px_fmt == 2 else 31)) * 255 // (63 if px_fmt == 2 else 31),
                              (v & 31) * 255 // 31))
            rows.append(bytes(row))

    def chunk(tag, data):
        c = tag + data
        return (len(data).to_bytes(4, "big") + c +
                (zlib.crc32(c) & 0xFFFFFFFF).to_bytes(4, "big"))

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", w.to_bytes(4, "big") + h.to_bytes(4, "big")
                   + bytes((8, 2, 0, 0, 0)))
           + chunk(b"IDAT", zlib.compress(b"".join(b"\x00" + r for r in rows), 6))
           + chunk(b"IEND", b""))
    pathlib.Path(path).write_bytes(png)

def describe(w, h, pitch, raw, px_fmt=1):
    """Is the frame one flat colour, and what colour?"""
    bpp = 4 if px_fmt == 1 else 2
    counts = {}
    for y in range(h):
        off = y * pitch
        for x in range(w):
            i = off + x * bpp
            if bpp == 4:
                px = (raw[i + 2], raw[i + 1], raw[i])
            else:
                value = raw[i] | raw[i + 1] << 8
                px = (((value >> (11 if px_fmt == 2 else 10)) & 31) * 255 // 31,
                      ((value >> 5) & (63 if px_fmt == 2 else 31)) * 255 // (63 if px_fmt == 2 else 31),
                      (value & 31) * 255 // 31)
            counts[px] = counts.get(px, 0) + 1
    top, n = max(counts.items(), key=lambda kv: kv[1])
    return {
        "distinct_colours": len(counts),
        "dominant_rgb": list(top),
        "dominant_fraction": round(n / (w * h), 4),
    }

if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(description="drive Genesis Plus GX headlessly and report frames")
    ap.add_argument("rom")
    ap.add_argument("--frames", type=int, default=240)
    ap.add_argument("--input", type=pathlib.Path, help="mdx-compatible controller replay JSON")
    ap.add_argument("--out", default="oracle_frame.png")
    ap.add_argument("--state2", default=None,
                    help="second frame count for a state diff against --state")
    ap.add_argument("--state", default=None,
                    help="dump the core's save state here and locate its VDP registers")
    ap.add_argument("--regs", action="store_true",
                    help="also report the 68k register file at the final frame")
    ap.add_argument("--ram", default=None,
                    help="also dump the core's 64 KiB of 68k work RAM here")
    ap.add_argument("--region", choices=["auto", "ntsc-u", "pal", "ntsc-j"], default="auto",
                    help="core region option; auto leaves the core's header detection unchanged")
    a = ap.parse_args()
    if a.region != "auto":
        variables["genesis_plus_gx_region_detect"] = a.region
    rom = pathlib.Path(a.rom)
    out = pathlib.Path(a.out)
    events = []
    if a.input:
        replay = json.loads(a.input.read_text())
        if set(replay) != {"events"} or not isinstance(replay["events"], list):
            raise SystemExit("input requires an events array")
        events = replay["events"]
        previous = -1
        ports = set()
        for event in events:
            frame, port = event["frame"], event.get("port", 0)
            if set(event) - {"frame", "port", "buttons"} or type(frame) is not int or frame < 0 or frame < previous or type(port) is not int or port not in (0, 1, 2, 3):
                raise SystemExit("invalid or unordered input event")
            if frame != previous: ports.clear()
            if port in ports or not set(event["buttons"]) <= set(LIBRETRO_BUTTONS.values()):
                raise SystemExit("duplicate port or unknown input button")
            previous = frame
            ports.add(port)
    f = run(rom.resolve(), a.frames, events)
    if a.state and a.state2:
        def cap(frames, path):
            run(rom.resolve(), frames, events)
            blob = save_state()
            pathlib.Path(path).write_bytes(blob)
            return blob
        b1 = cap(a.frames, a.state)
        print(json.dumps({"stage": "one", "frames": a.frames, "bytes": len(b1)}))
        b2 = cap(int(a.state2), a.state + ".2")
        # Restoring is not needed: run() reloads the ROM each time.
        runs = []
        start = None
        for i in range(min(len(b1), len(b2))):
            if b1[i] != b2[i]:
                if start is None:
                    start = i
            else:
                if start is not None and i - start <= 40:
                    runs.append((start, i, b1[start:i], b2[start:i]))
                start = None
        print(json.dumps({"changed_runs": len(runs)}))
        for a0, a1, x, y in runs[:24]:
            print("  @%7d  %s -> %s" % (a0, x.hex(), y.hex()))
    if a.state and not a.state2:
        blob = save_state()
        if blob is not None:
            state_path = pathlib.Path(a.state)
            state_path.parent.mkdir(parents=True, exist_ok=True)
            state_path.write_bytes(blob)
        off, regs = find_vdp_regs_in_state(blob)
        print(json.dumps({
            "state_bytes": len(blob) if blob else 0,
            "vdp_regs_offset": off,
            "vdp_regs": regs,
        }))
    if a.regs:
        regs = read_68k_registers()
        if regs is None:
            print(json.dumps({"error": "the core exposed no 68k register context"}))
            sys.exit(1)
        print(json.dumps({"registers": regs}))
    if a.ram:
        ram = read_work_ram()
        if ram is None:
            print(json.dumps({"error": "the core exposed no work RAM"}))
            sys.exit(1)
        out_ram = pathlib.Path(a.ram)
        out_ram.parent.mkdir(parents=True, exist_ok=True)
        out_ram.write_bytes(ram)
        print(json.dumps({"work_ram": str(out_ram), "bytes": len(ram),
                          "nonzero": sum(1 for b in ram if b)}))
    if f is None:
        print(json.dumps({"error": "the core produced no frame"}))
        sys.exit(1)
    w, h, pitch, raw = f
    write_png(out, w, h, pitch, raw, pixel_format[0])
    print(json.dumps({"frame": str(out), "frames": a.frames, "pixel_format": pixel_format[0], "samples": samples}))
