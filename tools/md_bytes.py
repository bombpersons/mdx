"""Normalize GPGX host-word storage without changing emulated CPU behaviour."""


def word_bytes(content, little_endian=True):
    """Return 68000 byte order from the pinned host-word buffer profile."""
    if len(content) % 2:
        raise ValueError("word buffer must have an even byte length")
    result = bytearray(content)
    if little_endian:
        result[::2], result[1::2] = result[1::2], result[::2]
    return bytes(result)
