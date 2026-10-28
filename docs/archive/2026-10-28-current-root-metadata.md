# Current bootstrap metadata component

Date: 2026-10-28. The normal Linux CLI built `metadata-current` with musl
binary SHA-256
`9da0f4ba3915b10534fcf6b5f131a9af486c6f9920fbfa57658600db668d37b8`.
The build took 34.18 seconds and published manifest
`sha256:6aae09c310c93c3690bbd944662fd15aaeff4971e7ca518f9e724fb5cb6db60f`.
It used the tracked metadata Containerfile and the reviewed helper bootstrap
whose archive SHA-256 is
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.

The converter component then used that validated OCI layout, the production
converter/agent binary and the exact raw bootstrap root
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
Its host-created OCI fixture records raw ACL and capability attributes from
the host kernel. Through the guest agent, all nine checks passed: observer
UID, ownership/mode, textual access ACL, raw access/default ACLs, user xattr,
raw file capability, symlink and removed file. The hardlink relation and
decoded file capability also passed. The component took 19.80 seconds.
The completed 114,542,592-byte root has SHA-256
`c14999033ffd29cef1318bd99db29727a46ad540b8e16e10c2a9b30e058d398d`.

This reproduces converter fidelity for the exact fixture on the current
bootstrap. It bypasses the production host receiver, jail and common claims.
The ordinary BuildKit OCI exporter still does not retain this recipe's
RUN-created ACL/user attributes; this component supplies their raw OCI
headers to isolate conversion. No general end-to-end ACL guarantee is claimed.

The committed earlier `acl-proof.py` was not edited. Its derived trial copy
changes only the repository location and expected bootstrap-root digest.
Derived source SHA-256:
`c862de87ab7b7ceffc68af960f7a4b2133c1689541df326b6cae8e985b830bd4`.
The normal CLI/component driver SHA-256 is
`d64276240b07fbf19476233b15073ca3dfd1eab55089eef657021bc37dc2609c`.

The 144,484,878-byte evidence bundle was copied out immediately:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/current-root-metadata-component.tar.gz`,
SHA-256 `1fbc569fd83b2e25b46874643ebf045f75926144397bf3e478b95ffba5c073da`.
It retains the driver, derived component source, expected/raw observations,
validated OCI layouts and opaque completed roots. It excludes the derivable
init/input disks. The converter work disk was deleted without host inspection.
No guest-writable filesystem was mounted or read on the host.
