import sys,zlib
d=open(sys.argv[1],'rb').read(); i=d.index(b'IKCFG_ST')+8
sys.stdout.write(zlib.decompressobj(31).decompress(d[i:]).decode())
