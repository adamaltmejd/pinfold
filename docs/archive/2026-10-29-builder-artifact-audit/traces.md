# Traces at 5c0ffa2

References: A = docs/ARCHITECTURE.md; C = crates/pinfold/src/cli.rs; M = crates/pinfold/src/main.rs; E = crates/e2e/tests/e2e; L = crates/e2e/src/lib.rs. Test references mean exercised, not necessarily a dedicated assertion. Absence means no direct occurrence found by rg; distinguish setup use from behavioral proof. CLI help/argv tests are prohibited by AGENTS.md, so gaps need spec judgment, not automatic new tests.

## Configuration
| Key | Code | Spec | Test exercise |
|---|---|---|---|
| profile/PINFOLD_PROFILE | config.rs:67,156,190 | A:1158 | E/pi.rs:1040 |
| image | config.rs:157 | A:1159 | E/pi.rs:1151,1158,1188 |
| containerfile | config.rs:158 | A:1162 | E/pi.rs:1166,1188 |
| allow/PINFOLD_ALLOW | config.rs:159,193 | A:1160 | E/pi.rs:730 |
| routes/PINFOLD_ROUTES | config.rs:160,194 | A:1161 | E/pi.rs:682 |
| cpus/PINFOLD_CPUS | config.rs:161,198 | A:1163 | E/pi.rs:731 |
| memory/PINFOLD_MEMORY | config.rs:162,205 | A:1164 | E/pi.rs:732 |
| PINFOLD_ENV_<NAME> | config.rs:140 | A:1165 | E/pi.rs:728,752 |
| PINFOLD_NO_UPDATE_CHECK | crates/pinfold/src/update.rs:67 (not config.rs) | A:1166,1003 | no direct e2e match |

## CLI
| Verb or flag | Code | Spec | Exercise |
|---|---|---|---|
| pi | M:65 | A:1199 | E/pi.rs:318 |
| attach | M:55 C:128 | A:1200 | no direct match |
| push | M:66 | A:1201 | implicit startup push E/pi.rs:345; explicit push no match |
| fetch | M:67 | A:1202 | E/pi.rs:435 |
| build --profile | M:56 C:1055 | A:1203 | L:284; E/pi.rs:588 |
| image build --containerfile --context --label --no-cache | C:1139,1219-1237 | A:173,1204 | E/box_.rs:1629-31,1825-29 |
| image rm | C:1140 | A:174,1204 | E/runtime.rs:468; E/box_.rs:1836 |
| allow | M:54 C:116 | A:1205 | L:310; E/pi.rs:404 |
| profile new --from | C:1252,1285 | A:1206 | E/box_.rs:2417 setup |
| clean --dry-run --unused | C:607,782-83 | A:1207 | E/runtime.rs:279,348 |
| doctor | C:824 | A:1208 | E/cli.rs:15 |
| artifacts | C:901 | A:1209 | E/box_.rs:3649 |
| apparmor-profile | C:912 | A:1210 | .github/workflows/ci.yml:56 setup; no direct e2e match |
| update --check | M:68; crates/pinfold/src/update.rs:178 | A:1211 | E/update.rs:16 tests update; no --check match |
| config ROOT | C:946 | A:1212 | E/pi.rs:1015; L:452 |
| box up | C:163 | A:162 | E/box_.rs:24 |
| box exec --tty -- | C:164,1001-02 | A:163 | L:376 non-tty; --tty no match |
| box stat | C:165 | A:164 | E/box_.rs:3555; L:404 |
| box down | C:166 | A:165 | E/box_.rs:24 |
| box list --label | C:170,1027 | A:166 | E/box_.rs:154 |
| box prune | C:171 | A:167 | E/runtime.rs:1108 |
| box inspect | C:172 | A:168 | E/runs.rs:29 |
| box events --limit --after | C:173,503,510 | A:169 | E/runs.rs:23,233 |
| box fetch --repo --into | C:174,395-96 | A:170 | E/box_.rs:853-60 |
| disk list | C:552 | A:171 | E/runtime.rs:983 |
| disk rm | C:576 | A:172 | E/runtime.rs:995 |
| init | M:52 | A:1214 | implicit in each box, e.g. E/box_.rs:24 |
| --version/-V | M:43 | A:1223 | E/cli.rs:165 only --version |
| --help/-h | M:47 C:97 | A:1223 | E/cli.rs:165 only box list --help |

## Guarantee to test
| Row | Spec | Test |
|---|---|---|
| 1 | A:1242 No network but loopback | E/box_.rs:1886 |
| 2 | A:1243 Only allowlisted hosts get through | E/box_.rs:1230 |
| 4 | A:1244 A route reaches exactly one host service | E/box_.rs:2036 |
| 5 | A:1245 Nothing can gain privileges | E/box_.rs:934 |
| 6 | A:1246 The environment is exactly the spec | E/pi.rs:93 |
| 8 | A:1247 Losing the owner fails closed | E/runtime.rs:999 |
| 9 | A:1248 The lifecycle works for a caller | E/box_.rs:24 |
| 10 | A:1249 Work leaves the box only by fetch | E/box_.rs:604 |
| 11 | A:1250 Disks persist and attach exclusively | E/runtime.rs:550 |
| 12 | A:1251 A changed project file stops the run | E/pi.rs:345 |
| 13 | A:1252 Project state persists and stays separate | E/pi.rs:204 |
| 15 | A:1253 Cleanup removes only pinfold's garbage | E/runtime.rs:30 |
| 17 | A:1254 up refuses before it creates | E/box_.rs:262 |
| 18 | A:1255 A caller reads the effective configuration as data | E/pi.rs:999 |
| 19 | A:1256 A caller-owned box launches the pinned harness | E/box_.rs:3637 |
| 20 | A:1257 A caller can tell an OOM kill from a failure | E/box_.rs:3555 |
| 21 | A:1258 An injecting route keeps the credential on the host | E/box_.rs:2278 |
| 23 | A:1259 A caller builds an image from its own tree | E/box_.rs:1460 |
| 26 | A:1260 A login route keeps the login on the host | E/box_.rs:2374 |
| 27 | A:1261 Doctor reports without changing state | E/cli.rs:15 |
| 28 | A:1262 Host updates are verified and atomic | E/update.rs:16 |
| 34 | A:1263 Blocked route responses release the upstream | E/box_.rs:2158 |
| 36 | A:1264 CONNECT tunnels share activity | scripts/slow-proxy.sh:26 and scripts/connect_tunnels_share_activity.py:413; separate .github/workflows/slow-proxy.yml |
| 38 | A:1265 Each run's history is its own | E/runs.rs:29 |
| 39 | A:1266 The launch record is the applied policy | E/pi.rs:593 |
| 42 | A:1267 Run records are retained within bounds | E/runtime.rs:1159 |
| 43 | A:1268 The VMM reaches only its own box | E/box_.rs:1086 |
