# Design — host-aware Codex automation routing

## Architecture

The change separates four concerns that are currently conflated by
`codex.dispatch_mode: herdr`:

1. **Surface detection** answers where the current Codex conversation is
   running (`cli`, `desktop`, or `unknown`) and records evidence.
2. **Policy resolution** maps a surface to a provider class (`herdr`, `dsh`,
   `codex`, or `ask`) without selecting a concrete resource.
3. **Conversation routing state** stores the user-selected executor and
   reviewer targets under the existing Codex conversation key.
4. **Provider validation/dispatch** checks the selected target and hands off to
   the already-defined Herdr or MCP DSH safety contract.

The controller remains responsible for interpreting an explicit user request,
persisting the selection, inspecting commands/diffs, and enforcing the review
gate. The hook only reports state; it never authorizes a worker.

## Surface evidence and precedence

`SurfaceEvidence` contains `surface`, `source`, and a safe human-readable
detail. Candidate evidence is ordered as follows:

1. An explicit surface value supplied by the current host or conversation
   routing command.
2. A host-specific App marker such as the configured Codex App-tools pipe.
3. A host-specific CLI marker supplied by the Codex CLI hook/runtime.
4. `unknown` when the evidence is absent, conflicting, or only consists of
   weak signals such as cwd, title, executable name, or missing App state.

The implementation must not expose environment values that could contain
tokens or private paths. It records marker names and source labels only.

The detector is deliberately injectable in tests. This permits deterministic
CLI/Desktop fixtures even when the current process is running inside Desktop,
and makes an upstream host-marker change a visible failing test rather than a
silent route change.

## Policy model

The config parser retains the legacy scalar modes and adds an `auto` policy
with a documented route table:

```yaml
codex:
  dispatch_mode: auto
  host_routes:
    cli: herdr
    desktop: dsh
    unknown: ask
```

`dispatch_mode` is a policy override, not a resource selection:

- `auto` uses `host_routes`;
- `inline` resolves to the current Codex provider;
- `herdr` resolves to the Herdr provider;
- `dsh` resolves to the MCP DSH provider;
- invalid values resolve to `ask` and emit a safe diagnostic in command-line
  inspection, never to an arbitrary executor.

The repository default becomes `auto`. Users who explicitly retain `herdr`
or `inline` keep the old topology. Host mapping selects only a provider class;
the target still needs a user choice where the provider requires one.

## Versioned routing state

Use a v2 schema at the same conversation-scoped path:

```json
{
  "version": 2,
  "platform": "codex",
  "context_key": "codex_<thread-id>",
  "surface": {
    "kind": "desktop",
    "source": "env_marker",
    "evidence": ["CODEX_APP_TOOLS_PIPE_PATH"]
  },
  "executor": {
    "provider": "dsh",
    "reference": "provider-managed",
    "label": "MCP DSH",
    "selected_by": "host_default"
  },
  "reviewer": null,
  "updated_at": "..."
}
```

`provider` and `reference` are the stable routing contract. Labels and
evidence are explanatory metadata, not identity. Provider adapters define how
to validate a reference:

- `codex/current` identifies this conversation;
- `herdr/<workspace>:<pane>` identifies a user-selected pane;
- `dsh/<provider-reference>` identifies a DSH-managed session or provider
  target;
- `chatgpt/<conversation>` identifies a selected web conversation;
- additional providers may use their own opaque references.

The state loader accepts the v1 shape and migrates it in memory: `inline`
becomes `codex/current`, `herdr` becomes a Herdr target, and the existing
ChatGPT reviewer becomes a ChatGPT target. The migration is lossless for
existing fields and does not rewrite historical task artifacts.

## Selection state machine

```text
user override
    ↓
valid persisted conversation target
    ↓
explicit config policy
    ↓
surface host default
    ↓
ask / fail closed
```

Executor and reviewer are independent slots. A missing or invalid slot is
reported together with any other missing slot in one selection prompt. An
explicit user replacement changes only the named slot. Validation failure
invalidates only the affected slot, never silently changes the other one.

The prompt lists the detected surface/evidence and provider choices, but never
auto-selects a pane, DSH worker, ChatGPT conversation, or model. It explicitly
accepts `executor=codex` and `reviewer=codex` as user-authorized self-routing.

## CLI and hook integration

`codex_routing.py` will expose provider-neutral operations:

- `discover`: print surface evidence, resolved policy, candidates, and missing
  target slots;
- `set-surface <kind>`: persist an explicit surface override;
- `set-executor --provider <name> --reference <value> [--label <text>]`;
- `set-reviewer --provider <name> --reference <value> [--label <text>]`;
- `clear-executor`, `clear-reviewer`, and `show`.

The per-turn hook imports the same resolver and emits a dynamic banner. It
must not describe every Codex turn as Herdr merely because the repository was
configured for the Herdr migration task. Workflow filtering uses stable
provider/surface blocks (`codex-auto`, `codex-herdr`, `codex-dsh`, and
`codex-inline`) while the banner supplies the concrete conversation routing
state.

## Compatibility and safety

- Existing `auto`, `sub-agent`, and `inline` semantics remain available.
- Existing explicit `herdr` state remains valid and is not silently changed.
- Host defaults never override an explicit user selection.
- Unknown/unsupported providers are representable for forward compatibility
  but cannot dispatch until a validating adapter is present.
- Provider validation is fail-closed; it does not use agent brand, pane
  position, cwd, title, recency, or a fixed reviewer URL as identity.
- DSH execution follows the existing clean-worktree, bounded wait, exact diff
  inspection, explicit apply, parent verification, and reviewer PASS protocol.
- Herdr execution follows the existing selected-pane, bounded wait, command
  approval, parent verification, and reviewer PASS protocol.

## Rollback

Revert the routing-only commit and restore the prior `herdr` configuration if
the host detector or provider adapters prove incompatible. The v2 runtime
files are conversation-scoped and may remain unread by the reverted code; no
MosDNS product/runtime state is touched.
