# Native `requery` (M1-c2)

Scope: the scheduled / triggered job that refreshes caches and remembered name lists by asking the host's own resolver for the recently seen
names again. Evidence: `docs/rust/validation-records/10-08-m1-c2/`. Reference: `plugin/executable/requery/requery.go`.

## Plugin and state file

`type: requery`, `args.file` (required; relative paths follow the configuration root). The file is the reference JSON document that the Vue UI also
edits through the API (`domain_processing`, `url_actions`, `scheduler`, `execution_settings`, `status`); it is written like Go's
`json.MarshalIndent` (two-space indent, HTML characters escaped, no trailing newline, RFC 3339 UTC times with the shortest sub-second part, the
zero time as `0001-01-01T00:00:00Z`, a never-set list as `null`) through a same-directory temporary file and a rename, so a failed write keeps the
previous file. A present file must parse (configuration error otherwise). Compiling and `check` write nothing; when the host starts serving
it creates a missing file (`idle`, `date_range_days: 30`), fills defaults (`url_call_delay_ms` 50, `queries_per_second` 100,
`date_range_days` 30, empty state `idle`) and turns a `running` state left by an interrupted run into `failed` with an end time. Files written
by the reference are read and answered identically, and the reference reads native files (`tools/m1c2-interop`, fixtures
`rust/native-host/tests/fixtures/requery/`).

## The task

Only one task runs at a time. A task: (1) `GET` each `save_rules` URL in order, `url_call_delay_ms` apart; (2) read the source files and merge
the names: `full:<name>` lines and statistics lines `count date name[|AD|CD|DO]` whose date is within `date_range_days` (UTC), exact
duplicates removed, missing files skipped; **no names: the task ends as `idle` and nothing is flushed**; (3) `GET` each `flush_rules`
URL; (4) ask `resolver_address` for the A and AAAA records of each name at `queries_per_second` names per second (UDP, a fresh socket and a 2 s
limit per query, answers ignored; AD/CD from the name's suffixes, DO as a 4096-byte OPT; at most 1024 queries in flight; names go through the
host's whole chain, which rebuilds cache and lists); (5) `GET` the `save_rules` URLs again. Progress is written every 100 names and at the end.
A URL step fails the task unless the answer is 2xx (`failed during <step> step: bad response from URL U: status N, body: B`); cancelling
during the query phase ends as `cancelled` (`task cancelled by user`), cancelling during a URL step or the merge fails the task
(`context canceled`), as in the reference. A host stop cancels a running task and records `cancelled` / `task cancelled by host shutdown`.

## URL actions (shared with `domain_output`)

`LocalApi::call` runs every URL action. A URL naming this host's API is served **in-process through the same plugin routes as the listener**
(rule providers, `webinfo`, `rewrite`, `domain_output`, `requery`, named caches), including the admission check and the management lease the
listener takes for mutating routes; a route it does not own (a switch) goes to the host's own listener over loopback; every other URL is an HTTP
request (30 s for `requery`, 5 s for `domain_output`). Which URLs are "this host" differs, as in the reference: `requery` treats a URL whose host is
exactly `127.0.0.1`, `localhost`, `0.0.0.0`, `::` or `::1` as the API whatever port it spells (the reference rewrites the port to the real one);
`domain_output`'s `domain_set_url` is local only when host and port equal the `api.http` address.

## Scheduler

Enabled, `interval_minutes > 0` and a `start_datetime` that parses as RFC 3339: the next run is the start time while it is ahead, else the next
multiple of the interval after it; the run is skipped while a task is running; reconfiguring re-arms. One timer exists at a time (the reference
leaves old timers alive after a reconfiguration).

## API (`/plugins/<tag>`)

`GET /` (whole document), `GET /status`, `POST /trigger` (200 / 409 `A task is already running.` / 500), `POST /cancel` (200 / 404
`No running task to cancel.`), `POST /scheduler/config` (`enabled`, `start_datetime`, `interval_minutes`, `date_range_days` — 400 `Invalid JSON
body`; the range changes only when positive; a failed save restores the in-memory settings and answers 500), `GET /stats/source_file_counts`
(saves first; per source the number of untrimmed lines that start with `full:`). Replies are `application/json; charset=utf-8`, compact, sorted
keys, trailing newline; wrong method 405, unknown path 404. While a managed apply retires the plugin, mutating routes answer 503.
The capability `cache.requery` is supported when a `requery` plugin exists.

## Known differences

Unbounded concurrency (1024 cap), single scheduler timer, names in first-seen order, a failed settings save restores memory, cancelled
state on host stop. The settings (`execution_settings`, `scheduler`) remain a UI-maintained runtime file until the typed settings layer (M1-f).
