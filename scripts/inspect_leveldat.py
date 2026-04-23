"""Inspect level.dat — decompress (gzip) and walk the NBT tree."""
import struct, sys, pathlib, gzip

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from inspect_chunk import R, NAMES, read_payload, TAG_END, TAG_COMPOUND

def main():
    path = sys.argv[1]
    maxd = int(sys.argv[2]) if len(sys.argv) > 2 else 6
    raw = pathlib.Path(path).read_bytes()
    data = gzip.decompress(raw)
    print(f"Decompressed {len(data)} bytes")
    r = R(data)
    root_tag = r.b()
    root_name = r.str()
    print(f"root: {NAMES[root_tag]} '{root_name}'")
    read_payload(r, root_tag, 0, maxd, "")

if __name__ == "__main__":
    main()
