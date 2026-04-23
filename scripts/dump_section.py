"""Dump the Palette IntArray and a few decoded blocks from a chunk section.

Usage: python dump_section.py <chunk.nbt> [section_index]
"""
import struct, sys, pathlib

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from inspect_chunk import (
    R, NAMES, TAG_END, TAG_BYTE, TAG_INT, TAG_BYTE_ARRAY, TAG_LIST, TAG_COMPOUND,
    TAG_INT_ARRAY, TAG_LONG_ARRAY, TAG_STRING, TAG_SHORT, TAG_LONG, TAG_FLOAT, TAG_DOUBLE,
)

def skip_payload(r, tag):
    if tag == TAG_BYTE: r.b()
    elif tag == TAG_SHORT: r.s()
    elif tag == TAG_INT: r.i()
    elif tag == TAG_LONG: r.L()
    elif tag == TAG_FLOAT: r.f()
    elif tag == TAG_DOUBLE: r.D()
    elif tag == TAG_BYTE_ARRAY:
        n = r.i(); r.p += n
    elif tag == TAG_STRING: r.str()
    elif tag == TAG_INT_ARRAY:
        n = r.i(); r.p += n*4
    elif tag == TAG_LONG_ARRAY:
        n = r.i(); r.p += n*8
    elif tag == TAG_LIST:
        etag = r.b(); n = r.i()
        for _ in range(n): skip_payload(r, etag)
    elif tag == TAG_COMPOUND:
        while True:
            t = r.b()
            if t == TAG_END: break
            r.str()
            skip_payload(r, t)


def find_sections(r):
    """Walk to Level.Sections list, return list of dicts: {field_name: (tag, raw_bytes_offset, payload_size)}."""
    root_tag = r.b(); r.str()  # root
    if root_tag != TAG_COMPOUND: return None
    # find Level
    while True:
        t = r.b()
        if t == TAG_END: return None
        name = r.str()
        if t == TAG_COMPOUND and name == 'Level':
            break
        skip_payload(r, t)
    # find Sections
    while True:
        t = r.b()
        if t == TAG_END: return None
        name = r.str()
        if t == TAG_LIST and name == 'Sections':
            break
        skip_payload(r, t)
    etag = r.b(); n = r.i()
    sections = []
    for _ in range(n):
        sec = {}
        while True:
            t = r.b()
            if t == TAG_END: break
            fname = r.str()
            if t == TAG_BYTE:
                sec[fname] = ('byte', r.b())
            elif t == TAG_BYTE_ARRAY:
                ln = r.i(); data = r.u(ln)
                sec[fname] = ('byte_array', ln, bytes(data))
            elif t == TAG_INT_ARRAY:
                ln = r.i(); ints = struct.unpack(f'>{ln}i', r.u(ln*4))
                sec[fname] = ('int_array', ln, list(ints))
            elif t == TAG_LONG_ARRAY:
                ln = r.i(); longs = struct.unpack(f'>{ln}q', r.u(ln*8))
                sec[fname] = ('long_array', ln, list(longs))
            else:
                skip_payload(r, t)
                sec[fname] = ('skipped', t)
        sections.append(sec)
    return sections


def main():
    path = sys.argv[1]
    sec_idx = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    data = pathlib.Path(path).read_bytes()
    r = R(data)
    sections = find_sections(r)
    if sections is None:
        print('No Sections found'); return
    print(f'Found {len(sections)} sections')
    if sec_idx >= len(sections):
        print(f'Section index {sec_idx} out of range'); return
    sec = sections[sec_idx]
    print(f'Section[{sec_idx}] fields: {list(sec.keys())}')
    if 'Y' in sec: print(f'  Y = {sec["Y"][1]}')
    if 'Palette' in sec:
        p = sec['Palette'][2]
        print(f'  Palette[{len(p)}]:')
        for i, e in enumerate(p):
            block_id = e >> 4
            meta = e & 0xF
            print(f'    [{i:3}] = {e:8d}  -> block_id={block_id}, meta={meta}')
    if 'Blocks' in sec:
        b = sec['Blocks'][2]
        print(f'  Blocks[{len(b)}]: first 16 bytes = {b[:16].hex()}')
    if 'Data' in sec:
        d = sec['Data'][2]
        print(f'  Data[{len(d)}]: first 16 bytes = {d[:16].hex()}')
    # Decode the first 16 blocks at (x=0..15, y=sec_idx*16, z=0)
    if 'Palette' in sec and 'Blocks' in sec and 'Data' in sec:
        b = sec['Blocks'][2]
        d = sec['Data'][2]
        p = sec['Palette'][2]
        print(f'  First block-row decode (y_local=0, z=0):')
        for x in range(16):
            i = x  # y=0, z=0
            hi = b[i]
            data_byte = d[i // 2]
            lo = (data_byte & 0x0F) if (i & 1) == 0 else ((data_byte >> 4) & 0x0F)
            pidx = (hi << 4) | lo
            if pidx >= len(p):
                print(f'    x={x}: hi={hi}, lo={lo}, pidx={pidx} OUT OF PALETTE (len={len(p)})')
            else:
                state = p[pidx]
                print(f'    x={x}: hi={hi}, lo={lo}, pidx={pidx}, state={state} -> id={state>>4}, meta={state&0xF}')


if __name__ == '__main__':
    main()
