# Box user name and Bash prompt (#335), 2026-11-20

The box user is now named `pinfold`, and built-in images give interactive
Bash the prompt `pinfold:\w\$ `. This file records why the mechanism was
chosen and what the debug binary did on macOS and on Linux.

## Mechanism

- At each start, pinfold generates `passwd` and `group` entries in
  `/var/lib/extrausers`. Built-in images install `libnss-extrausers`. They
  look up users with `files extrausers` and groups with
  `extrausers files`.
- On Linux, PID 1 writes the entries into the root overlay, where it
  already writes `resolv.conf`, before the root becomes read-only. A
  failure there is ignored and the uid stays unnamed. The BOOT request
  gains one string: the literal `HOME`, or empty. The builder sends it
  empty.
- On Apple, the host writes the entries into the box's state dir and
  mounts that directory read-only. The mount is skipped when a caller
  mount or disk overlaps the path.

Groups are looked up in extrausers first because macOS's gid 20 (`staff`)
is `dialout` in Debian. With `files` first, `id` reported
`gid=20(dialout)`.

Users are looked up in `files` first because of a fault in
libnss-extrausers 0.6-6. With `extrausers files`, the Linux box gave:

```
passwd 0: pinfold:x:1000:1000:pinfold:/home/pi:/bin/bash
passwd 42: _apt:x:42:65534::/nonexistent:/usr/sbin/nologin
passwd 1000: pinfold:x:1000:1000:pinfold:/home/pi:/bin/bash
passwd 4242: none
group 0: root:x:0:
```

`id root` printed `uid=0(pinfold)`, and `ls -l` showed root's files as
owned by `pinfold`. Group lookups were correct. The first macOS run, which
used the old order, showed the same fault: the root-owned Apple mount top
was listed as `pinfold root`.

### Rejected

- **Guest init only.** The Apple root is read-only, and init runs as the
  host uid with no capabilities, so it cannot write `/etc` or mount.
- **A host file bind over `/etc/passwd`.** Apple `container` 1.7.1
  refuses file sources. Probe:
  `container run --rm --read-only --mount type=bind,source=$PWD/probe/passwd,target=/etc/passwd,readonly debian:trixie-slim ...`
  printed `Error: path '.../probe/passwd' is not a directory`.
- **A host directory over `/etc`.** It would hide the image's `/etc` and
  the always-applied empty `resolv.conf`.
- **A symlinked `/etc/passwd` in the image.** Image builds would see a
  dangling file, and so would derived recipes.
- **A writable tmpfs location.** This would add a writable path beyond
  Always applied's list.
- **nss_wrapper with `LD_PRELOAD`.** It needs a library in the image and
  an environment variable on every exec. extrausers is a plain NSS module.
- **Writing `/etc/passwd` directly on Linux.** That would name the uid in
  any Linux image, but Apple cannot do the same. One location on both
  runtimes keeps one rule.

### Prompt

`/etc/bash.bashrc` sets `PS1='pinfold:\w\$ '`.

- Debian's `/etc/bash.bashrc` already returns early for non-interactive
  shells.
- Debian's `/etc/profile` sources it for login Bash.
- A home `.bashrc` runs after it, so its `PS1` wins.
- Nothing is written into the home.

Plain `bash -l` reads `~/.profile`, not `~/.bashrc`. That is stock Bash
behaviour.

## Verification on macOS (Apple container 1.7.1)

The debug binary from this branch was used. The wrapper `$T/pf` set
`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` under a fresh
`$T`, plus `PINFOLD_PROFILE=default` and `VISUAL=true EDITOR=true`.

The bundled images were not rebuilt. The test image was
`localhost/pinfold-t335:latest`, built with `container build` from this
recipe:

```
FROM localhost/pinfold/profile-default:latest
RUN <the lines this change adds to default.Containerfile's base stage>
RUN find / -xdev -perm /6000 -type f -exec chmod a-s {} +
```

The project configs took the profile from `PINFOLD_PROFILE` (`default`)
and set `image = "localhost/pinfold-t335:latest"`, so the default
profile's home seeds applied.

The real recipe's base stage was also built:
`container build --target base -t localhost/pinfold-t335-base:latest -f default.Containerfile`.
It had the nsswitch lines, `PS1='pinfold:\w\$ '` as the last line of
`/etc/bash.bashrc`, and `/usr/lib/libnss_extrausers.so.2`. This build used
the first lookup order. The final order was built on Linux (below).

### Final lookup order, share project

`pf -- sh -s < check.sh` printed:

```
--- whoami
pinfold
--- id
uid=501(pinfold) gid=20(pinfold) groups=20(pinfold)
--- id root
uid=0(root) gid=0(root) groups=0(root)
--- getent passwd
root:x:0:0:root:/root:/bin/bash
_apt:x:42:65534::/nonexistent:/usr/sbin/nologin
pinfold:x:501:20:pinfold:/home/pi:/bin/bash
--- getent group
root:x:0:
pinfold:x:20:
--- sh echo ~
/home/pi
--- nsswitch
passwd:         files extrausers
group:          extrausers files
--- account dir
drwxr-xr-x 4 root    root    128 Nov 20 10:12 .
-rw-r--r-- 1 pinfold pinfold  14 Nov 20 10:12 group
-rw-r--r-- 1 pinfold pinfold  44 Nov 20 10:12 passwd
-rw-r--r-- 1 root root 839 Nov  4 21:23 /etc/passwd
--- write test
touch: cannot touch '/var/lib/extrausers/passwd': Read-only file system
--- ownership
-rw-r--r-- 1 501 20 0 Nov 20 10:12 /home/pi/made-in-home
-rw-r--r-- 1 501 20 0 Nov 20  2026 .../t335b/share/made-in-project
```

### First lookup order: share and disk projects, prompt, output bytes

These runs used `extrausers files` for users too. The fault above affects
only uid 0, and the prompt and output checks do not depend on the order.

Share project (`kind = "share"`):

- `pf -- whoami` printed `pinfold` and exited 0.
- `id` printed `uid=501(pinfold) gid=20(pinfold) groups=20(pinfold)`.
- `getent passwd 501` printed `pinfold:x:501:20:pinfold:/home/pi:/bin/bash`.
- `sh -c 'echo ~'` printed `/home/pi`.
- New files in the project and in the home were `501 20` in the box.
- On the host, the project file showed `501 0` under `ls -ln`. A file
  made by the unchanged image showed the same. macOS gives a new file its
  parent directory's group, which is `wheel` here, so ownership is
  unchanged.

Interactive runs used `expect` with `pf -- bash`, then `pf -- bash -l`.
Each sent `whoami`, `cd ~` and `echo PS1=[$PS1]`. Output, with
bracketed-paste escapes left out:

```
pinfold:/private/tmp/.../t335.qV3X/share$ whoami
pinfold
pinfold:/private/tmp/.../t335.qV3X/share$ cd ~
pinfold:~$ echo PS1=[$PS1]
PS1=[pinfold:\w\$ ]
pinfold:~$ exit
```

`bash -l` showed the same prompts and ended with `logout`.

The custom prompt was written in the box with
`printf "%s\n" "PS1='custom-prompt\\$ '" >> ~/.bashrc`. After that,
`pf -- bash` showed `custom-prompt$ ` at every prompt and printed
`PS1=[custom-prompt\$ ]`. The home's existing `.claude`, `.codex`, `.pi`
and `.bash_history` were still there.

Non-interactive output was captured with stderr dropped:

- `pf -- bash -c 'printf x' > out1`: exit 0; `od -c` showed one byte, `x`.
- `pf -- bash -lc 'printf y' > out2`: one byte, `y`.
- `pf -- true > out3`: 0 bytes.

Disk project (`kind = "disk"`, not a Git checkout):

- The identity, lookup, `~` and ownership lines matched the share run.
- The file made in the project was `501 20` on the disk.
- `pf -- bash` showed `pinfold:/private/tmp/.../t335.qV3X/diskp$ `, and
  `whoami` printed `pinfold`.

### Images without extrausers

The share project was rerun with two images:

- `image = "localhost/pinfold/profile-default:latest"`, the unchanged
  bundled image.
- `image = "debian:trixie-slim"`.

Both started and exited 0, and the account directory was still mounted.
`id` printed `uid=501 gid=20(dialout) groups=20(dialout)`. `whoami`
printed `cannot find name for user ID 501` and exited 1.

## Verification on Linux (dev VM, Debian 13, x86_64)

The branch's tree was copied to the VM and built with
`cargo build -p pinfold --locked --target x86_64-unknown-linux-musl`.

The wrapper used throwaway XDG roots. The project was a fresh Git repo
with one commit, using the `disk` mount from `config edit`. `pf build`
built `default` from the real recipe. Its builder booted with the new
BOOT field. The build was repeated after the lookup order changed.

Results with the final recipe:

```
--- id
uid=1000(pinfold) gid=1000(pinfold) groups=1000(pinfold)
--- getent passwd 0 / 42 / 1000
root:x:0:0:root:/root:/bin/bash
_apt:x:42:65534::/nonexistent:/usr/sbin/nologin
pinfold:x:1000:1000:pinfold:/home/pi:/bin/bash
--- getent group 0 / 1000
root:x:0:
pinfold:x:1000:
--- id root
uid=0(root) gid=0(root) groups=0(root)
--- sh echo ~
/home/pi
--- account dir
-rw-r--r-- 1 root root 16 Nov 20 10:11 group
-rw-r--r-- 1 root root 47 Nov 20 10:11 passwd
--- write test
touch: cannot touch '/var/lib/extrausers/passwd': Permission denied
--- ownership
-rw-r--r-- 1 1000 1000 0 ... /home/agent/t335/x/proj/made-in-project
-rw-r--r-- 1 1000 1000 0 ... /home/pi/made-in-home
--- prompt
PS1='pinfold:\w\$ '
```

- `pf -- whoami` printed `pinfold`.
- `pf -- bash -c 'printf x' > out` wrote one byte.
- `pf -- bash -i -c 'echo "PS1=[$PS1]"'` printed `PS1=[pinfold:\w\$ ]`.
- Under `script`, `pf -- bash` showed `pinfold:/home/agent/t335/x/proj$ `.

The problem existed on Linux too: the host uid 1000 has no entry in
Debian's `/etc/passwd`.

## Not run

- A generic `box up` without a literal `HOME`, which should give home `/`.
- The Apple overlap skip.
- On Linux, the custom-PS1 and `bash -l` cases.

## Cleanup

The following were removed:

- the macOS project volumes, by name;
- the test images `localhost/pinfold-t335:latest` and
  `localhost/pinfold-t335-base:latest`;
- the temp roots;
- the VM's disks (`disk rm --all` under the throwaway root) and its copy
  of the tree.

`container ls -a`, `container volume ls` and `container image ls` showed
none of them afterwards.
