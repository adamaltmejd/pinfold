#!/bin/bash
# Each spec is fed to `box up`, whose stdin then closes; prints its stdout and exit code.
img=localhost/pinfold/profile-default:latest
try() {
  echo "== $1"
  echo "$2" | "$P" box up
  echo "exit $?"
}
try "repeated name" '{"name":"hl-b1","image":"'$img'","harnesses":["pi","pi"]}'
try "unknown name" '{"name":"hl-b2","image":"'$img'","harnesses":["pi","gemini"]}'
try "old harness key" '{"name":"hl-b3","image":"'$img'","harness":"pi"}'
try "login names an unlisted harness" '{"name":"hl-c1","image":"'$img'","harnesses":["pi"],"egress":{"routes":{"c":{"login":"claude","from":"HL_DUMMY_CLAUDE"}}}}'
try "two claude login routes" '{"name":"hl-c2","image":"'$img'","harnesses":["claude"],"egress":{"routes":{"c1":{"login":"claude","from":"HL_DUMMY_CLAUDE"},"c2":{"login":"claude","from":"HL_DUMMY_CLAUDE"}}}}'
try "two claude login routes beside pi" '{"name":"hl-c3","image":"'$img'","harnesses":["pi","claude"],"egress":{"routes":{"c1":{"login":"claude","from":"HL_DUMMY_CLAUDE"},"c2":{"login":"claude","from":"HL_DUMMY_CLAUDE"},"p":{"login":"pi","provider":"anthropic","from":"HL_DUMMY_ANTHROPIC"}}}}'
