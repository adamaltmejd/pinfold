#!/usr/bin/env python3
"""Hostile box streams for the probe: rewrite a bundle's header lines.

  craft.py stream LISTING BUNDLE OUT          listing, empty line, bundle
  craft.py rehead BUNDLE OUT SIG [LINE...]    new signature line, then LINEs
                                              before the original header body
  craft.py rename BUNDLE OUT OLD NEW          rename a head in the header
"""
import sys


def split(path):
    data = open(path, "rb").read()
    head, sep, pack = data.partition(b"\n\n")
    return head.split(b"\n"), pack


def main(cmd, *args):
    if cmd == "stream":
        listing, bundle, out = args
        open(out, "wb").write(open(listing, "rb").read() + b"\n" + open(bundle, "rb").read())
    elif cmd == "rehead":
        bundle, out, sig, *extra = args
        lines, pack = split(bundle)
        new = [sig.encode()] + [e.encode() for e in extra] + lines[1:]
        open(out, "wb").write(b"\n".join(new) + b"\n\n" + pack)
    elif cmd == "rename":
        bundle, out, old, new = args
        lines, pack = split(bundle)
        lines = [l.replace(b" " + old.encode(), b" " + new.encode()) if not l.startswith(b"#") else l
                 for l in lines]
        open(out, "wb").write(b"\n".join(lines) + b"\n\n" + pack)


main(*sys.argv[1:])
