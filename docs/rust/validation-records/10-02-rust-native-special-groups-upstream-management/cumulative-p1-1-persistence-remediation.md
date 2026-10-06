# Cumulative P1-1 persistence remediation — round 2

The exact prior object `ccc147bb35aab2301633325c563368ac8457dbba` received
explicit same-ID P1-1 / FAIL: native save drops known origin, so an ordinary
restart produces an originless hit. That finding is valid. The first remediation
record's memory-only limitation is insufficient for newly written native dumps;
its former claim is retained as historical evidence, not accepted final behavior.

## Fix

Native snapshots and prepared entries now carry the same immutable attachment.
The native codec writes known ResponseOrigin in optional protobuf entry field 7;
existing fields 1–6, gzip name and block framing remain unchanged. Existing Go
protobuf readers skip the extension. The versioned bounded metadata participates
in entry/block/owned decode limits; malformed, duplicate, unsupported or oversized
metadata fails before import merge. Truly legacy dumps lack the field and keep
unknown origin. No new cache sidecar, owner, protocol, write gate or transaction
is introduced: the extension travels in the existing atomically persisted dump,
so empty-dump invalidation and durable flush clear origin with the answer.

## RED / GREEN

The new real group DNS regression warms a normalized cache, stops the host through
its normal save/drain path, creates a fresh host from the same fixture, and checks
the hit's actual entry/peer/transport, empty attempts and no added ECS extension.
It fails against the prior code with `None` versus `persisted_supplier`, then
passes with the new codec. Three initial harness compile errors (constructor,
private method and missing serialization trait) are preserved and are not RED
behavior evidence; the fourth attempt is the valid failing regression.

Final isolated fmt, strict workspace all-targets Clippy and native build pass.
Native-host 381/0/3, libraries 410/0/3, and complete workspace 1,184/0/3 pass
(79 targets; three subprocess probe entrypoints explicitly run by parents).
[Command results](evidence/cumulative-p1-1-final-checks-restart-final1.json) and
[counts](evidence/cumulative-p1-1-persistence-counts.json) preserve the logs.

Real controlled UDP/TCP DNS plus HTTP explicitly save three group caches, stop
the native process normally and start a fresh process from the same committed
fixture. All three restart queries hit, preserving actual entry/peer/transport
and empty attempts; total peer requests remain six before and after restart.
[Restart proof](evidence/cumulative-p1-1-live-restart/restart-proof.json) retains
actual audit output. All proof processes were stopped. Existing Go protobuf
reader successfully reads each native dump's legacy fields and its unknown
extension; [probe source](evidence/cumulative-origin-go-probe.go),
[result](evidence/cumulative-origin-go-probe-result.json) and
[output](evidence/cumulative-origin-go-probe.log) are public.

527 local/isolated source inputs match; eleven source inputs differ from the
accepted S7 source. Prior Vue/Go inputs and browser evidence remain unchanged.
The separate reader probe and live runner are retained executable evidence.
No stage or prior object PASS substitutes for the cumulative re-review result.
