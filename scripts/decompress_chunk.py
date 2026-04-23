"""Quick helper to extract raw NBT from an MCA for inspection."""
import struct, zlib, sys, pathlib

mca = sys.argv[1]
cx = int(sys.argv[2]) if len(sys.argv) > 2 else 0
cz = int(sys.argv[3]) if len(sys.argv) > 3 else 0
out = sys.argv[4] if len(sys.argv) > 4 else "chunk.nbt"

data = pathlib.Path(mca).read_bytes()
i = cx + 32*cz
b0,b1,b2,b3 = data[i*4:(i+1)*4]
offset = (b0<<16 | b1<<8 | b2)
print(f"Chunk ({cx},{cz}) sector offset={offset} count={b3}")
if offset == 0:
    print("Chunk not present"); sys.exit(0)
fo = offset * 4096
length = struct.unpack('>I', data[fo:fo+4])[0]
comp = data[fo+4]
print(f"Length={length} compression={comp}")
payload = data[fo+5:fo+4+length]
if comp == 2:
    nbt = zlib.decompress(payload)
elif comp == 1:
    import gzip; nbt = gzip.decompress(payload)
else:
    print("Unsupported compression"); sys.exit(1)
pathlib.Path(out).write_bytes(nbt)
print(f"Wrote {len(nbt)} bytes → {out}")
