"""Streaming signed stereo PCM capture, independent of any emulator library."""
import array
import ctypes
import math
import struct
import sys
import wave


class WaveCapture:
    def __init__(self, path, first_frame, last_frame):
        if not 0 <= first_frame < last_frame:
            raise ValueError('audio range must be a nonempty zero-based frame interval')
        self.path = path
        self.first_frame, self.last_frame = first_frame, last_frame
        self.frame = -1
        self.writer = None
        self.sample_rate = None
        self.frames = self.nonzero = self.peak = self.squares = 0

    def configure(self, sample_rate):
        rate = round(sample_rate)
        if rate < 1 or abs(sample_rate - rate) > 0.001:
            raise ValueError('WAV capture requires an integer sample rate')
        if self.writer is not None:
            if rate != self.sample_rate:
                raise ValueError('sample rate changed during capture')
            return
        self.sample_rate = rate
        self.writer = wave.open(str(self.path), 'wb')
        self.writer.setnchannels(2)
        self.writer.setsampwidth(2)
        self.writer.setframerate(rate)

    def begin_frame(self, frame):
        self.frame = frame

    def batch(self, data, frames):
        if self.first_frame <= self.frame < self.last_frame and frames:
            raw = ctypes.string_at(data, frames * 4)
            if sys.byteorder != 'little':
                words = array.array('h', raw)
                words.byteswap()
                raw = words.tobytes()
            self._write(raw)
        # libretro requires the count consumed even when a range excludes it.
        return frames

    def single(self, left, right):
        if self.first_frame <= self.frame < self.last_frame:
            self._write(struct.pack('<hh', left, right))

    def _write(self, raw):
        if self.writer is None:
            raise RuntimeError('audio format must be configured before samples')
        self.writer.writeframesraw(raw)
        values = array.array('h', raw)
        if sys.byteorder != 'little':
            values.byteswap()
        self.frames += len(values) // 2
        self.nonzero += sum(x != 0 for x in values)
        self.peak = max(self.peak, max(map(abs, values), default=0))
        self.squares += sum(x * x for x in values)

    def close(self):
        if self.writer is not None:
            self.writer.close()
            self.writer = None

    def report(self):
        return {'sample_rate':self.sample_rate,'channels':2,'bits_per_sample':16,
                'sample_frames':self.frames,'duration_seconds':self.frames / self.sample_rate,
                'nonzero_samples':self.nonzero,'peak':self.peak,
                'rms':math.sqrt(self.squares / max(1, self.frames * 2))}
