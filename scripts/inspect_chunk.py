"""Minimal NBT walker — prints the tag tree of a raw uncompressed NBT blob.

Usage: python inspect_chunk.py <nbt_file> [max_depth]
"""
import struct, sys, pathlib

TAG_END = 0
TAG_BYTE = 1
TAG_SHORT = 2
TAG_INT = 3
TAG_LONG = 4
TAG_FLOAT = 5
TAG_DOUBLE = 6
TAG_BYTE_ARRAY = 7
TAG_STRING = 8
TAG_LIST = 9
TAG_COMPOUND = 10
TAG_INT_ARRAY = 11
TAG_LONG_ARRAY = 12

NAMES = {v: k for k, v in globals().items() if k.startswith("TAG_")}

class R:
    def __init__(self, data):
        self.d = data; self.p = 0
    def u(self, n): s=self.d[self.p:self.p+n]; self.p+=n; return s
    def b(self): return self.u(1)[0]
    def s(self): return struct.unpack('>h', self.u(2))[0]
    def i(self): return struct.unpack('>i', self.u(4))[0]
    def L(self): return struct.unpack('>q', self.u(8))[0]
    def f(self): return struct.unpack('>f', self.u(4))[0]
    def D(self): return struct.unpack('>d', self.u(8))[0]
    def str(self):
        n=self.s() if False else struct.unpack('>H', self.u(2))[0]
        return self.u(n).decode('utf-8', errors='replace')

def read_payload(r, tag, depth, max_depth, indent):
    if tag == TAG_BYTE: return r.b()
    if tag == TAG_SHORT: return r.s()
    if tag == TAG_INT: return r.i()
    if tag == TAG_LONG: return r.L()
    if tag == TAG_FLOAT: return r.f()
    if tag == TAG_DOUBLE: return r.D()
    if tag == TAG_BYTE_ARRAY:
        n = r.i(); data = r.u(n); return (n, data)
    if tag == TAG_STRING: return r.str()
    if tag == TAG_INT_ARRAY:
        n = r.i(); r.p += n*4; return f"[{n} ints]"
    if tag == TAG_LONG_ARRAY:
        n = r.i(); r.p += n*8; return f"[{n} longs]"
    if tag == TAG_LIST:
        etag = r.b(); n = r.i()
        out = f"List<{NAMES.get(etag,etag)}>[{n}]"
        if depth < max_depth:
            for k in range(min(n, 3)):
                v = read_payload(r, etag, depth+1, max_depth, indent+"  ")
                if etag == TAG_COMPOUND:
                    pass  # already printed
                else:
                    print(f"{indent}  [{k}]: {short_repr(etag, v)}")
            # skip remaining
            for k in range(min(n,3), n):
                _ = read_payload(r, etag, depth+1, max_depth, indent+"  ")
        else:
            for _ in range(n):
                read_payload(r, etag, depth+1, max_depth, indent)
        return out
    if tag == TAG_COMPOUND:
        print(f"{indent}{{")
        while True:
            t = r.b()
            if t == TAG_END: break
            name = r.str()
            if t == TAG_COMPOUND:
                print(f"{indent}  {name}:")
                read_payload(r, t, depth+1, max_depth, indent+"  ")
            elif t == TAG_LIST:
                v = read_payload(r, t, depth+1, max_depth, indent+"  ")
                print(f"{indent}  {name}: {v}")
            else:
                v = read_payload(r, t, depth+1, max_depth, indent+"  ")
                print(f"{indent}  {name} ({NAMES.get(t,t)}): {short_repr(t, v)}")
        print(f"{indent}}}")
        return None

def short_repr(tag, v):
    if tag == TAG_BYTE_ARRAY:
        n, data = v
        return f"ByteArray[{n}]: {bytes(data[:min(16,n)]).hex()}..."
    return repr(v)

def main():
    path = sys.argv[1]
    maxd = int(sys.argv[2]) if len(sys.argv) > 2 else 8
    data = pathlib.Path(path).read_bytes()
    r = R(data)
    root_tag = r.b()
    root_name = r.str()
    print(f"root: {NAMES[root_tag]} '{root_name}'")
    read_payload(r, root_tag, 0, maxd, "")

if __name__ == "__main__":
    main()
