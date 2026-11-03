# Route-name collision refusal

Issue #229. The released v0.5.1 CLI accepted `Fixture.internal` and
`fixture.internal` in one spec, reached READY and sent the lowercase request
to the latter fixture. The targets and credentials were not shown to cross.

Plan validation now rejects duplicate names under the proxy's existing ASCII
lowercase normalization. G17 adds that refusal and observes no box/state.
Its existing race winner supplies the positive control: two distinct routes
return their separate host-fixture bytes, including one mixed-case name.
No new box start, helper, test row, dependency or binary seam was added.
Removing the uniqueness check makes the refusal become READY; no existing
G17 block observes this collision. Expected reason/exit come from the
contract, and response bytes come from the host fixtures.

Independent correctness/security and lean/test-admission reviews passed the
final source. Format and locked all-target Clippy passed. The full ordinary
Mac suite passed 26/0/1 ignored in 97.75s. G34 remains in its separate slow
gate. Linux checks are recorded on the PR; no timing or platform claim is
inferred from the Mac result.

## Caller changes

Specs whose route names differ only by ASCII case now refuse as `spec`
before creation instead of silently selecting one target.
