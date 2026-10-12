"""Independent authored-data checks for exporter/converter observer semantics."""
import json
import pathlib
import struct
import tempfile
import unittest
from md_bytes import word_bytes
from mdx_trace_to_binary import convert
from oracle_vdp_snapshot import normalize


class ObserverTests(unittest.TestCase):
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
