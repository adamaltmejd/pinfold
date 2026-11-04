# Apple lifecycle investigation

One reviewed external scheduling experiment used clean main `fe00731`,
the ordinary debug CLI (SHA256
`57b709aba88ea03f898ec52a0b5878767a239d8b2aaf24dc827216c165e597c9`),
and Apple Container 1.6.1. No product or test assertions changed.

A healthy box created and cleanly detached one disposable disk. During a
second start, the owner record and run directory identified the final
native startup phase. Its direct client was paused by verified PID, parent,
uid, start time and executable name. Native inventory showed no named box.
The owner was killed and reaped; the claim retained its generation.

One public prune emitted `pruned` for that exact name and owner, then the
claim and native instance were absent. Source-supported inference: a native
instance became visible to prune after the empty snapshots, despite the
paused client. There is no retained direct native-generation snapshot.
After resume, the client exited. The disk remained unattached and unclean;
one removal succeeded. A final scoped snapshot about 118 seconds after
cleanup found no owned native box, volume or claim.

This did not capture creation after prune completed. The observation loop
ended at client exit, not after a 30-second quiet interval. Its false
`generation_accounted` value means no instance remained to compare, not an
accounting failure. No read-only reattachment or concurrent lock-probe case
ran. The historical #167 refusal and #207 volume holder remain unexplained;
#202 remains open. No product fix is justified by this experiment.

Independent source/driver and result reviews passed. Receipts are retained
at `/private/tmp/pinfold-lifecycle-once-result.json` and
`/private/tmp/pinfold-lifecycle-final-inventory.json`; the driver is beside
them as `pinfold-lifecycle-once.py`. Only scoped metadata was retained.
