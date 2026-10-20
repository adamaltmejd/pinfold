# Bundles drop branches the host already reaches (2026-10-20)

Found while building `box fetch` (#135), git 2.56 on the host and 2.47
in the default image.

- `git bundle create B --branches ^OID…` leaves out every branch whose
  tip is reachable from the negated commits. A box branch created at a
  commit the host already fetched is not in the bundle, so a
  `git fetch B +refs/heads/*:refs/remotes/NAME/*` never lands it.
- When every tip is reachable, `bundle create` refuses: "Refusing to
  create empty bundle", exit 128. A second fetch with nothing new fails
  the same way.
- So the box lists its branches before the bundle. The host fetches the
  bundle's objects with fsck, then writes all refs in one `update-ref
  --stdin` transaction. A listed branch the bundle omits lands only when
  `git rev-list -n1 OID --not --glob=refs/remotes/NAME/*` is empty.
- A bundle with `@filter=blob:none` passes `git bundle verify`. Its
  fetch fails the connectivity check and changes no ref, but leaves a
  `.promisor` pack in the host repository. The host now reads the
  bundle header and fails a filtered bundle before verify.
- `git fetch` with no destination needs explicit sources: the glob
  `refs/heads/*` alone is an invalid refspec. Sources go in on
  `--stdin`, so the branch count never meets argv limits.
