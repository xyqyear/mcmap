"""Extract FML.ItemData from level.dat as JSON {id: name}.

Usage: dump_ids.py <level.dat> <out.json>
"""
import struct, sys, pathlib, gzip, json

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from inspect_chunk import R

TAG_END, TAG_BYTE, TAG_SHORT, TAG_INT, TAG_LONG = 0, 1, 2, 3, 4
TAG_FLOAT, TAG_DOUBLE = 5, 6
TAG_BYTE_ARRAY, TAG_STRING, TAG_LIST = 7, 8, 9
TAG_COMPOUND, TAG_INT_ARRAY, TAG_LONG_ARRAY = 10, 11, 12

def skip_payload(r, tag):
    if tag == TAG_BYTE: r.p += 1
    elif tag == TAG_SHORT: r.p += 2
    elif tag == TAG_INT: r.p += 4
    elif tag == TAG_LONG: r.p += 8
    elif tag == TAG_FLOAT: r.p += 4
    elif tag == TAG_DOUBLE: r.p += 8
    elif tag == TAG_BYTE_ARRAY:
        n = struct.unpack('>i', r.u(4))[0]; r.p += n
    elif tag == TAG_STRING:
        n = struct.unpack('>H', r.u(2))[0]; r.p += n
    elif tag == TAG_INT_ARRAY:
        n = struct.unpack('>i', r.u(4))[0]; r.p += n*4
    elif tag == TAG_LONG_ARRAY:
        n = struct.unpack('>i', r.u(4))[0]; r.p += n*8
    elif tag == TAG_LIST:
        et = r.b(); n = struct.unpack('>i', r.u(4))[0]
        for _ in range(n): skip_payload(r, et)
    elif tag == TAG_COMPOUND:
        while True:
            t = r.b()
            if t == TAG_END: return
            n = struct.unpack('>H', r.u(2))[0]; r.p += n
            skip_payload(r, t)

def read_compound_extract(r, want):
    """Read a compound, extract named fields into 'want' dict, skip others."""
    while True:
        t = r.b()
        if t == TAG_END: return
        n = struct.unpack('>H', r.u(2))[0]
        name = r.u(n).decode('utf-8', errors='replace')
        if name in want:
            if t == TAG_STRING:
                n2 = struct.unpack('>H', r.u(2))[0]
                want[name] = r.u(n2).decode('utf-8', errors='replace')
            elif t == TAG_INT:
                want[name] = struct.unpack('>i', r.u(4))[0]
            else:
                skip_payload(r, t)
        else:
            skip_payload(r, t)

def find_compound(r, path):
    """Navigate into a nested compound at root compound → path entries."""
    while True:
        t = r.b()
        if t == TAG_END: return False
        n = struct.unpack('>H', r.u(2))[0]
        name = r.u(n).decode('utf-8', errors='replace')
        if path and name == path[0]:
            if t == TAG_COMPOUND:
                if len(path) == 1:
                    return True
                if find_compound(r, path[1:]):
                    return True
            else:
                skip_payload(r, t)
                continue
        else:
            skip_payload(r, t)

def main():
    src = pathlib.Path(sys.argv[1])
    dst = pathlib.Path(sys.argv[2])
    raw = src.read_bytes()
    data = gzip.decompress(raw)
    r = R(data)
    root_tag = r.b(); n = struct.unpack('>H', r.u(2))[0]; r.p += n  # root unnamed

    # Walk to FML. ItemData
    # root is compound; find FML compound
    while True:
        t = r.b()
        if t == TAG_END:
            raise SystemExit("FML not found")
        nn = struct.unpack('>H', r.u(2))[0]
        name = r.u(nn).decode('utf-8', errors='replace')
        if name == "FML" and t == TAG_COMPOUND:
            break
        skip_payload(r, t)

    # Inside FML compound: find ItemData list
    while True:
        t = r.b()
        if t == TAG_END:
            raise SystemExit("ItemData not found")
        nn = struct.unpack('>H', r.u(2))[0]
        name = r.u(nn).decode('utf-8', errors='replace')
        if name == "ItemData" and t == TAG_LIST:
            break
        skip_payload(r, t)

    etag = r.b(); count = struct.unpack('>i', r.u(4))[0]
    assert etag == TAG_COMPOUND, f"expected compound list, got {etag}"
    print(f"ItemData has {count} entries")

    blocks = {}
    items = {}
    for _ in range(count):
        want = {"K": None, "V": None}
        read_compound_extract(r, want)
        k, v = want["K"], want["V"]
        if k is None or v is None: continue
        if k and k[0] == '\x01':
            blocks[str(v)] = k[1:]
        elif k and k[0] == '\x02':
            items[str(v)] = k[1:]

    print(f"Blocks: {len(blocks)}, Items: {len(items)}")
    dst.write_text(json.dumps({"blocks": blocks, "items": items}, indent=2, sort_keys=True))
    print(f"Wrote {dst}")

if __name__ == "__main__":
    main()
