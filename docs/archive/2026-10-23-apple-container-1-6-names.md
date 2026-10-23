# Local image names under Apple container 1.6.0 (#166)

Homebrew upgraded Apple `container` to 1.6.0 on the Mac mini on
2026-10-22. Its `image list --format json` names images built as
`pinfold/<repo>:<tag>` by their normalized form,
`docker.io/pinfold/<repo>:<tag>`. A short name is Docker Hub shorthand.

Effect on pinfold 0.2.7 on macOS, reproduced through the CLI:
`pinfold image build repro166 …` then `pinfold image rm repro166` printed
`"ids":[]` and both tags stayed. Caller-image retention found no caller
sources, and `config` reported `image_built: false` for a built image.

A second effect appeared in the suite. A `pinfold/profile-default:latest`
stored under 1.5.0 kept its short name and shadowed the new
`docker.io/…:latest`. A short ref resolved to the pre-upgrade image after
every rebuild.

Decision (Adam): name every pinfold repo `localhost/pinfold/…`. Checked on
1.6.0: `build --tag localhost/pinfold/x:latest`, `image list` (lists that
name unchanged), `inspect`, `run`, `FROM localhost/pinfold/x:latest`, `tag`
and `delete` all resolve locally; `pinfold/x:latest` does not alias to it.
A missing `localhost/` image is pulled only from localhost, which is refused
("Connection refused" after about 43 s), never from Docker Hub. podman
already lists local builds under `localhost/`, so both runtimes report
the name pinfold tags. No migration: old short-named images were removed by
hand.
