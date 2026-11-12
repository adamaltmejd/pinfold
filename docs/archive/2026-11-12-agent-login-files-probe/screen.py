"""Print the text of a terminal capture: escape sequences stripped, blank
and repeated lines dropped, the last 70 lines."""

import re
import sys

raw = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
text = re.sub(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[PX^_][^\x1b]*\x1b\\|\x1b[@-Z\\-_]|\x1b.", "\n", raw)
seen, lines = set(), []
for line in re.split(r"[\r\n]+", text):
    line = "".join(c for c in line if c.isprintable()).strip()
    if line and line not in seen:
        seen.add(line)
        lines.append(line)
print("\n".join(lines[-70:]))
