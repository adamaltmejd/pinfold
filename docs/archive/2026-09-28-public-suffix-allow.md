# Suffix allow entries on public suffixes

A `.suffix` allow entry on a domain where anyone can get a subdomain
(`.github.io`, `.workers.dev`, `.amazonaws.com`) lets a box reach a host an
attacker controls. pinfold accepted such entries silently. The default list
has no suffix entries, so this guards a caller's or project's own list.

Source: boxd's egress allowlist refuses these wildcards
(https://docs.boxd.sh/guides/egress, read 2026-09-28). boxd uses a
hand-kept list; pinfold uses the Public Suffix List instead, so the rule
has one source and no judgement per domain.

Rule: refuse `.X` when X is a public suffix or has one below it. Checked
against the list on 2026-09-28 (16,501 lines):

| Entry | Public suffix | Suffixes below | Result |
|---|---|---|---|
| `.github.io`, `.vercel.app`, `.workers.dev`, `.pages.dev`, `.cloudfront.net`, `.githubusercontent.com`, `.googleapis.com` | yes | 0 | refused |
| `.amazonaws.com` | no | 561 | refused |
| `.com` | yes | 1,254 | refused |
| `.github.com`, `.npmjs.org`, `.anthropic.com`, `.openai.com` | no | 0 | allowed |

"Below it" is what catches `.amazonaws.com`, which is not itself on the
list. Exact names are not checked: `my-bucket.s3.amazonaws.com` names one
party.

Not taken from boxd: substituting a bound secret anywhere in a request
(header, query, body). A body sent to a write API on the bound host, such
as a GitHub gist, would publish the real value. pinfold's injecting routes
set named headers only.
