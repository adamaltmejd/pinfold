# CI fixture-cost trials

Adam approved three fixture changes after a read-only review. Reference release
runs passed all 26 Linux tests but took 310.822681613 and 302.862042162 seconds.
The target is at least 20 seconds saved, with 280 seconds or less as useful margin.

No assertion block failed the test-audit bar. No test or assertion is deleted.
Each change received independent correctness and lean/test-audit review:

- G15: move its existing image-removal scenario before clean purges the builder
  cache. Live/idle cleanup, foreign resources, in-use image retention and root
  reclamation remain proved. G44 retains its separate post-cache-removal launch.
- G11/G43: explicitly allocate one CPU to selected Linux fixture boxes. Host
  owners and VMs still race, concurrent readers remain separate, and all disk,
  interruption, namespace, descriptor and owner-death assertions stay. Mac specs,
  omitted-CPU fixtures, G39's file-two/environment-one proof and builder resources
  are unchanged. No global environment or shared box-helper policy is added.
- G27: after its private-cache read-only checks, use a separate ordinary fixture
  environment and the existing shared image for the box control. Remove the
  dedicated scratch-image build. Keep restriction=1, the unprofiled-copy path,
  runtime/userns-apparmor refusal and the same profiled READY control.

The existing named sabotages still make each scenario fail. Expected values
come from the fixture, spec, host kernel and real runtime. Guards retain their
full fixture lifetime. Production code, pins, profiles and timer are unchanged.

| Trial | Linux result | Budget clock | Mac result |
| --- | --- | ---: | --- |
| Reorder, 6242321 | 26/0/1 ignored; CI41317536643 | 308.189129727s, budget FAIL | 24/0/1 ignored, 95.76s |
| Plus CPUs, 48871b8 | 26/0/1 ignored; CI41318131652 | 299.784452152s, PASS | 24/0/1 ignored, 80.42s |
| Plus shared doctor image | Final ordinary CI pending at preparation | Pending | 24/0/1 ignored, 76.77s |

The first two runs do not establish the 20-second target or isolate causality.
Final results belong to PR256. No unchanged retry, budget waiver, merge or
release is part of these trials. Assertion-macro counts in the affected files
remain box_.rs 269, runtime.rs 92 and cli.rs 12. At preparation, E2E Rust has
8,311 lines and product Rust under crates/pinfold/src has 18,289 lines.
