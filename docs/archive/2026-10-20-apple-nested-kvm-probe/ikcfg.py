# Prototype for the 2026-10-20 Apple nested KVM probe. Not pinfold code.
# Prints the kernel config embedded after IKCFG_ST in a kernel image.
import sys,zlib
d=open(sys.argv[1],'rb').read(); i=d.index(b'IKCFG_ST')+8
sys.stdout.write(zlib.decompressobj(31).decompress(d[i:]).decode())
