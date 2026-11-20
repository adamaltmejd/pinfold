# Box user name and Bash prompt (#335), 2026-11-20

The default image now names the usual host uids `pinfold` and gives
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
    for account in 501:20 1000:1000; do \
        uid=${account%:*}; gid=${account#*:}; \
        if ! getent passwd "$uid" >/dev/null; then \
            echo "pinfold:x:$uid:$gid:pinfold:/home/pi:/bin/bash" >> /etc/passwd; \
        fi; \
    done; \
    if ! getent group 1000 >/dev/null; then echo 'pinfold:x:1000:' >> /etc/group; fi; \
    printf '%s\n' "PS1='pinfold:\\w\\\$ '" >> /etc/bash.bashrc
```

- uid 501, gid 20 is the first macOS user. uid 1000, gid 1000 is the usual
  first Linux user.
- Each line is added only if Debian has no account at that uid. A group is
  added only for gid 1000. gid 20 stays Debian's `dialout`.
- The home is `/home/pi`, the project box's `HOME` (`pi/state.rs`).

Both lines share the name `pinfold`. Lookups by uid, which `whoami`, `id`,
Bash and `ls` use, each find their own line. A lookup by name
(`getent passwd pinfold`, `~pinfold`) returns the first line, uid 501.
Both lines have the same home, so `~pinfold` is right for either uid. A
name such as `pinfold1000` would avoid the duplicate but would show in
`whoami` on every Linux box.

The prompt does not use `\u`, so a uid with no name never sees
`I have no name!`. Debian's `/etc/bash.bashrc` already returns early when
not interactive, and `/etc/profile` sources it for login Bash. A home
`.bashrc` runs later, so its `PS1` wins. Nothing is written into homes.

Other uids keep working without a name.

## Verification

### macOS (Apple container 1.7.1)

The binary was a debug build with no pinfold source changes. The wrapper
`$T/pf` set `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` under
a fresh `$T`, plus `PINFOLD_PROFILE=default` and `VISUAL=true EDITOR=true`.

The bundled images were not rebuilt. The test image,
`localhost/pinfold-t335:latest`, was the existing
`localhost/pinfold/profile-default:latest` plus the `RUN` above. The
project was a share with `image = "localhost/pinfold-t335:latest"`.

`container run --user U` on the test image:

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
`pf -- bash` showed `custom-prompt$ ` at each prompt.

`pf -- bash -c 'printf x' > out` exited 0, and `od -c` showed one byte,
`x`.

With `image = "localhost/pinfold/profile-default:latest"`, an image
without the lines:

- the box started and exited 0;
- `id` printed `uid=501 gid=20(dialout)`;
- `whoami` printed `cannot find name for user ID 501`.

### Linux (dev VM, Debian 13, x86_64)

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
