# Box user name and Bash prompt (#335), 2026-11-20

The default image now names uid 501 `pinfold` and uid 1000 `pi`, and gives
interactive Bash the prompt `pinfold:\w\$ `. The binary is unchanged.
This file records the first design, why it was dropped, and the final
image-only design and its verification.

## First design (dropped)

At each start, pinfold generated `passwd` and `group` entries for the host
uid and gid in `/var/lib/extrausers`. The default image installed
`libnss-extrausers` and looked there.

- **Linux:** PID 1 wrote the entries into the root overlay. The literal
  `HOME` went into the BOOT request as the account's home.
- **Apple:** the host wrote them into the box's state dir and mounted that
  directory read-only. Apple `container` refuses file bind mounts, so
  `/etc/passwd` could not be replaced directly.

It worked on both runtimes. It also showed a fault in libnss-extrausers
0.6: placed before `files`, it answered uid 0 with its own first entry,
so `id root` printed `uid=0(pinfold)`.

The operator review found six control-level problems:

- PID 1's write before `pivot_root` follows symlinks the image plants.
- The Apple bind's overlap check is lexical. Image symlinks bypass it, so
  the bind can shadow `/tmp` or land in a writable share.
- A literal `HOME` in the Linux BOOT request contradicts "the spec's env
  reaches the guest only over the exec connection".
- The bind can stop boxes that started before.
- The bind is missing from the launch record.
- The design needed uid and gid exceptions.

A cosmetic feature does not justify a new mount, a boot field and a PID 1
write. Every change outside `profile/` and the docs was reverted.

## Final design

`profile/image/default.Containerfile` gains one `RUN` in its base stage:

```
RUN set -eu; \
    if ! getent passwd 501 >/dev/null && ! getent passwd pinfold >/dev/null; then \
        useradd -M -N -u 501 -g 20 -d /home/pi -s /bin/bash pinfold; \
    fi; \
    if ! getent group 1000 >/dev/null && ! getent group pi >/dev/null; then \
        groupadd -g 1000 pi; \
    fi; \
    if ! getent passwd 1000 >/dev/null && ! getent passwd pi >/dev/null; then \
        useradd -M -N -u 1000 -g 1000 -d /home/pi -s /bin/bash pi; \
    fi; \
    printf '%s\n' "PS1='pinfold:\\w\\\$ '" >> /etc/bash.bashrc
```

- uid 501, gid 20 is the first macOS user, named `pinfold`. uid 1000,
  gid 1000 is the usual first Linux user, named `pi`, with group `pi`.
- Each account or group is added only if Debian leaves both its id and its
  name free. gid 20 stays Debian's `dialout`.
- The home is `/home/pi`, the project box's `HOME` (`pi/state.rs`).
- `useradd` also writes locked `/etc/shadow` entries.
- Other uids keep working without a name.

The first image-only round gave both uids the name `pinfold`. Review
rejected that:

- `id -u "$(whoami)"` printed 501 when running as 1000.
- Debian's `pwck` treats duplicate names as an error.

The prompt does not use `\u`, so a uid with no name never sees
`I have no name!`. Debian's `/etc/bash.bashrc` returns early when not
interactive. Nothing is written into homes. ARCHITECTURE.md states only
the default prompt. Whether a home `PS1` wins depends on which startup
files the shell reads. It was observed for plain interactive `bash` only
(below).

## Verification

### macOS (Apple container 1.7.1)

The binary was a debug build with no pinfold source changes. The wrapper
`$T/pf` set `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` under
a fresh `$T`, plus `PINFOLD_PROFILE=default` and `VISUAL=true EDITOR=true`.

The bundled images were not rebuilt. The test image,
`localhost/pinfold-t335:latest`, was the existing
`localhost/pinfold/profile-default:latest` plus the `RUN` above. The
project was a share with `image = "localhost/pinfold-t335:latest"`.

#### Final names

The test image was the existing `localhost/pinfold/profile-default:latest`
plus the final `RUN`. It was run with `container run --rm --network none
--read-only --tmpfs /tmp --user U`:

```
== --user 501:20
pinfold
uid=501(pinfold) gid=20(dialout) groups=20(dialout)
id -u $(whoami) = 501
pinfold:x:501:20::/home/pi:/bin/bash
PS1=[pinfold:\w\$ ]
== --user 1000:1000
pi
uid=1000(pi) gid=1000(pi) groups=1000(pi)
id -u $(whoami) = 1000
pi:x:1000:1000::/home/pi:/bin/bash
PS1=[pinfold:\w\$ ]
```

Each block's last line came from `bash -ic 'echo PS1=[$PS1]'`.

As root, `pwck -r` reported no duplicate. It listed only missing home
directories: Debian's own `man`, `lp`, `news`, `uucp`, `www-data`, `list`
and `irc`, plus `pi` (`/home/pi` is a disk mount point, not part of the
image). It exited 2. The stock `profile-default` image gives the same
exit status and the same lines apart from `pi`. `grpck -r` exited 0.

#### First image-only round (both uids named `pinfold`)

The CLI runs below used this round's image. The shared name does not
affect the prompt, the home, ownership or captured output.

`container run --user U` on that image:

```
501:20     uid=501(pinfold) gid=20(dialout) groups=20(dialout); whoami pinfold; ~ /home/pi
1000:1000  uid=1000(pinfold) gid=1000(pinfold) groups=1000(pinfold); whoami pinfold; ~ /home/pi
4242:4242  uid=4242 gid=4242 groups=4242; whoami: cannot find name for user ID 4242
root       uid=0(root) gid=0(root) groups=0(root)
```

`getent passwd pinfold` printed the uid 501 line.

`pf -- sh -s < check.sh` printed:

```
--- whoami
pinfold
--- id
uid=501(pinfold) gid=20(dialout) groups=20(dialout)
--- getent passwd 501
pinfold:x:501:20:pinfold:/home/pi:/bin/bash
--- echo ~
/home/pi
--- ownership
-rw-r--r-- 1 501 20 0 Nov 20 10:22 /home/pi/made-in-home
-rw-r--r-- 1 501 20 0 Nov 20  2026 .../t335d/share/made-in-project
```

On the host, the project file showed `501 0`. macOS gives a new file its
parent directory's group (`wheel`). The unchanged image showed the same
in the first round.

The interactive runs used `expect`, sending `whoami`, `cd ~` and `exit`.
Bracketed-paste escapes are left out:

```
pinfold:/private/tmp/.../t335d/share$ whoami
pinfold
pinfold:/private/tmp/.../t335d/share$ cd ~
pinfold:~$ exit
```

`pf -- bash -l` showed the same prompts and ended with `logout`.

`PS1='custom-prompt\$ '` was then appended to the home's `.bashrc`.
Plain interactive `pf -- bash` then showed `custom-prompt$ ` at each
prompt. This was not checked for login shells.

`pf -- bash -c 'printf x' > out` exited 0, and `od -c` showed one byte,
`x`.

With `image = "localhost/pinfold/profile-default:latest"`, an image
without the lines:

- the box started and exited 0;
- `id` printed `uid=501 gid=20(dialout)`;
- `whoami` printed `cannot find name for user ID 501`.

### Linux (dev VM, Debian 13, x86_64)

This run used the first image-only round, in which uid 1000 was also
named `pinfold`. It was not repeated with `pi`. The `container run --user
1000:1000` check above covers the final name.

The tree was copied to the VM, and `cargo build -p pinfold --locked
--target x86_64-unknown-linux-musl` built it. With throwaway XDG roots,
`pf build` built `default` from the real recipe. The project was a fresh
Git repo with a disk mount.

`pf -- sh -c 'whoami; id; getent passwd "$(id -u)"; echo ~; tail -1 /etc/bash.bashrc'`
printed:

```
pinfold
uid=1000(pinfold) gid=1000(pinfold) groups=1000(pinfold)
pinfold:x:1000:1000:pinfold:/home/pi:/bin/bash
/home/pi
PS1='pinfold:\w\$ '
```

Under `script`, `pf -- bash` showed `pinfold:/home/agent/t335/x/proj$ `,
and `whoami` printed `pinfold`.

### Cleanup

On macOS, the share project's home volume, the test image and the temp
roots were removed. `container ls -a`, `container volume ls` and
`container image ls` showed none of them afterwards.

On the VM, the disks were removed with `disk rm --all` under the throwaway
root, and the copied tree was deleted.
