# Design

## Evidence and review boundary

Parse the existing attempt-3 `RESULT_JSON` as evidence, verify the twelve DNS response and peer-delta cases plus source/binary identity, service baseline, process exits, and cleanup, then preserve a sanitized compact artifact under the original canary task. Record its source file hash; omit private config payloads and credentials. Commit the evidence correction so the reviewer can inspect it with read-only Git comparison. Submit one exact committed range from the parent of the original canary commit through the correction head, with its parent/head SHAs and changed paths; name the original commit as an internal boundary. Disclose the different target-change reviewer and absent original snapshot. The new authorization covers future review only.

## Start gate

Use task metadata `automation_required=true` to identify tasks using `automation.py authorize/activate`; set it on the replacement review task. Before `task.py start` writes status or the session pointer, check a valid authorization snapshot for the exact task/context, frozen units and transport evidence, and reviewer equality with the persisted context. Share this validation with `activate`. Fail without mutation. Non-automation tasks keep existing behavior.

## Supersession

Add `task.py supersede <old> <replacement> --reason ...`. Require an existing replacement with recorded review acceptance. Write terminal `status=superseded`, successor and reason; clear pointers to the old task. Preserve the task directory and evidence. Listing and context must distinguish it from active tasks and successful archives.

## Compatibility

No Rust or Go runtime changes. Tests use temporary repositories through the public CLI and automation API. If the reviewer rejects the evidence, retain the finding and stop; do not rerun the canary automatically.
