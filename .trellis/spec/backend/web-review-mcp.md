# Web Review Transport (Playwright MCP)

The detailed review-loop contract — roles, the review request/response contract,
the pending cadence, and the PASS/FAIL rules — is in
[`quality-guidelines.md`](./quality-guidelines.md). This file defines the
**transport** used to reach the reviewer, so those rules can be executed without
guessing which tool to use.

## Scope

Applies to web ChatGPT planning/root review and review-message operations when a
project task requires the selected ChatGPT conversation to review repository
work. It covers only how the controller connects to and talks to that
conversation. It does not change the review contract, the executor routing, or
any task authorization.

## Configured transport

Use the configured Playwright MCP server. It is declared in the user's Codex
configuration and must be used as configured; do not substitute a different
browser automation path.

| Setting | Value |
|---|---|
| Server declaration | `[mcp_servers.playwright]` in `~/.codex/config.toml` |
| Command | `npx` |
| Arguments | `-y @playwright/mcp@latest --extension --profile-dir-name=Profile 1` |
| Chrome profile directory | `Profile 1` |
| Profile visible name | `Ai Agent` |

The profile directory is the disambiguator: the extension connects to the Chrome
profile whose directory is `Profile 1`, whose visible name is `Ai Agent`. Always
verify the target ChatGPT tab through the Playwright tab list/DOM before sending
anything; a different profile can hold a different signed-in account.

## First connection or reconnect

On the first connection and after any reconnect, the Playwright Extension page
may show that `codex-mcp-client` is trying to connect to the Playwright
Extension. Complete the handshake before any review message:

1. Open the Playwright Extension page and read the pending request from
   `codex-mcp-client`.
2. Click **Allow & select**.
3. Wait until the page reports **Connected to Playwright client** or
   **codex-mcp-client connected**.
4. Verify the target ChatGPT tab through the Playwright tab list/DOM — confirm it
   is the selected MosDNS project conversation, not just any ChatGPT tab.

Use a bounded connection check: poll at a small fixed interval with a hard
overall deadline, and stop as soon as the connected state or the deadline is
reached. Do not busy-wait, and do not spin on the extension page indefinitely.

## Conversation selection

Use the user-selected MosDNS project conversation as the review destination. When
the user asks for a new project conversation, create one and use it for that
review round. Do not silently switch to a different conversation, and do not
reuse an unrelated one.

## Review message contents

Every review message must contain all of the following, so the reviewer never has
to infer missing state:

- repository, branch, and full commit hash;
- GitHub URL or exact commit/tree path;
- changed-file scope;
- validation evidence (commands and results);
- current task/phase status;
- explicit forbidden follow-on scope (for example: no production wiring, no
  unrelated implementation, no task start); and
- a request for an explicit `PASS` / `FAIL` decision.

## PASS/FAIL discipline

Never infer `PASS` from a successful push, an active or streaming UI, an
unchanged preview, or silence. Only an explicit reviewer `PASS` counts. A pending
or active turn is a wait state; wait with the bounded cadence defined in
`quality-guidelines.md` and make no speculative changes while waiting.

## Transport failure

Do not use CUA/Computer Use as the normal review transport. If Playwright MCP
cannot connect after the bounded troubleshooting flow in this document, **stop
and report the missing MCP transport** to the user instead of silently switching
to another mechanism. A silent transport switch makes it impossible to tell
whether a review was actually delivered.

## Credentials and permissions

Never copy tokens, cookies, passwords, session identifiers, or account data into
repository docs, task artifacts, commit messages, or review messages. Grant only
the permissions the connection flow requires; do not accept unrelated browser
permissions requested during the handshake.

## Why / Failure prevention

- **Transport ambiguity causes unreviewed work.** Without a recorded transport,
  the controller can believe a review was sent when no message reached the
  conversation, and then treat a green local check as acceptance.
- **Profile ambiguity reaches the wrong account.** More than one Chrome profile
  can be signed in. Naming `Profile 1` / `Ai Agent` and verifying the tab through
  the Playwright tab list/DOM prevents reviewing in the wrong session.
- **An incomplete handshake looks like a hung review.** The extension's
  "trying to connect" state is not connected; the explicit **Allow & select**
  step plus a bounded wait for the connected state distinguishes a real pending
  review from a connection that never established.
- **Silent transport fallback hides delivery failure.** If a fallback path is
  used without saying so, a missing message and a pending review become
  indistinguishable — so the rule is to stop and report instead.
- **Leaked credentials are irreversible.** Anything written into repository docs,
  commits, or a shared conversation is effectively published.
- **Inferred PASS skips the gate.** Push success, an active UI, and silence are
  all states in which the reviewer has said nothing.

## Good vs Wrong

**Good**

```text
1. Connect through the configured Playwright MCP server:
   [mcp_servers.playwright] -> npx -y @playwright/mcp@latest
     --extension --profile-dir-name=Profile 1
2. Extension page shows codex-mcp-client requesting access -> click
   Allow & select.
3. Wait (bounded) for "Connected to Playwright client".
4. Verify via the Playwright tab list that the target tab is the selected
   MosDNS project conversation (profile dir Profile 1, name Ai Agent).
5. Send one message containing: repo, branch, full commit, GitHub URL,
   changed scope, validation evidence, task status, forbidden follow-on scope,
   and a request for explicit PASS/FAIL.
6. Wait/read at the documented bounded cadence for an explicit PASS or FAIL.
```

**Wrong**

```text
- Click through the extension prompt and send immediately, without waiting for
  the connected state.
- Send the review to whatever ChatGPT tab happens to be focused.
- Write "PASS assumed" because the push succeeded and the conversation looks
  active.
- Fall back to CUA/Computer Use (or any other automation) without telling the
  user that Playwright MCP was unavailable.
- Paste a session cookie or account token into the message or into a task file
  "for context".
- Accept a broad browser permission request that the connection flow never asked
  for.
```
