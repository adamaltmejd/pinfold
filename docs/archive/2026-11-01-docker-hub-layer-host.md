# Docker Hub layer download host

Nightly run 41292650603 failed during a real Linux default-profile build.
BuildKit resolved Debian's manifest, then received Forbidden while downloading
a layer from `docker-images-prod.s3.dualstack.us-east-1.amazonaws.com`.
The host was absent from the default build allowlist. The run's egress receipt
was not retained, so the proxy denial reason was not independently observed.
Shared fixture setup then poisoned its mutex, causing secondary failures.

Adam approved adding this exact host. Linux build defaults and the architecture
contract now include it. Exact hostname matching, agent egress, proxy checks,
tests and the 300-second CI budget are unchanged. Existing Linux end-to-end
setup exercises the real default-profile build. Docker Hub can choose another
download host, so a passing build alone does not prove it used this host.

The previous nightly, 41184753850, passed all 26 tests but took 310.950684999
seconds. That separate budget failure remains valid; this change does not
address suite timing or authorize a release.
