"""Independent authored-data checks for exporter/converter observer semantics."""
import json
import ctypes
import pathlib
import struct
import tempfile
import unittest
from md_bytes import word_bytes
from mdx_trace_to_binary import convert
from oracle_vdp_snapshot import normalize
from libretro_abi import AvInfo, Geometry, Timing, Variable


class ObserverTests(unittest.TestCase):
    def test_libretro_av_and_variable_layout_match_the_public_c_abi(self):
        self.assertEqual(ctypes.sizeof(Geometry),20)
        self.assertEqual(ctypes.sizeof(Timing),16)
        self.assertEqual(AvInfo.timing.offset,24)
        self.assertEqual(ctypes.sizeof(AvInfo),40)
        info=AvInfo(); info.timing.fps=60.0; info.timing.sample_rate=48000.0
        self.assertEqual(bytes(info)[24:],struct.pack("dd",60.0,48000.0))
        variable=Variable(b"test",None); variable.value=b"value"
        self.assertEqual(variable.key,b"test");self.assertEqual(variable.value,b"value")
    def test_logical_word_bytes(self):
        self.assertEqual(word_bytes(bytes.fromhex("ff011234")), bytes.fromhex("01ff3412"))
        self.assertEqual(word_bytes(bytes.fromhex("01ff"), False), bytes.fromhex("01ff"))
        with self.assertRaises(ValueError):
            word_bytes(b"\x00")

    def test_saved_state_exports_normalized_wram(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            offset = 16 + 65536 + 8192 + 1 + 4 + 16 + 1024
            raw = bytearray(offset + 65536 + 128 + 128 + 32)
            raw[:16] = b"GENPLUS-GX 1.7.6\0"
            raw[16:18] = bytes.fromhex("ff01")
            source = root / "state"; source.write_bytes(raw)
            result = normalize(source, root / "snapshot")
            content = pathlib.Path(result["memory_artifacts"]["wram"]["path"]).read_bytes()
            self.assertEqual(len(content), 65536)
            self.assertEqual(content[:2], bytes.fromhex("01ff"))

    def test_trace_state_order_and_bad_registers(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            event = {"pc": 512, "sr": 0x2700, "d": list(range(8)), "a": list(range(8, 16))}
            source = root / "trace.jsonl"; source.write_text(json.dumps(event)+"\n")
            result = convert(source, root / "trace.bin")
            self.assertEqual(result["records"], 1)
            self.assertEqual(struct.unpack("<18I", (root / "trace.bin").read_bytes()), (512, 0x2700, *range(16)))
            event["pc"] = -1; source.write_text(json.dumps(event)+"\n")
            with self.assertRaises(ValueError):
                convert(source, root / "bad.bin")
            self.assertFalse((root / "bad.bin").exists())


if __name__ == "__main__":
    unittest.main()
