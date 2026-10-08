# Native `domain_output` (M1-c1)

Scope: the plugin that counts the query names of a sequence, persists them in the reference file formats, publishes the generated
rule list to a `domain_set_light` and injects each new name into a `domain_mapper` at once. Evidence:
`docs/rust/validation-records/10-08-m1-c1/`. Reference: `plugin/executable/domain_output/domain_output.go` and
`plugin/data_provider/domain_mapper/domain_mapper.go` (`QuickAdd`).

## Arguments

All optional: `file_stat`, `file_rule`, `gen_rule` (paths relative to the configuration root; empty = file not written), `pattern`,
`appended_string`, `max_entries`, `dump_interval` (seconds, `<= 0` means 60), `domain_set_url`, `enable_flags`, `target_mapper`,
`target_mark` (0..=63; 0 sets no flag), `target_tag`. Unknown arguments, a `target_mapper` that is not a `domain_mapper` of the
configuration and a mark above 63 are configuration errors. The quick form `exec: domain_output a,b,...` is not supported.

## Recording

The plugin is an executable (`exec: $tag`); it never changes the query or its routing and never fails it. The key is the query name
without the trailing dot, ASCII-lower-cased, plus `|AD` and/or `|CD` (in that order) when `enable_flags` is on and the query has those
header bits. The EDNS DO bit is never part of a key (the reference checks it on a query whose OPT the server replaced already, so it is
always false there; production files contain only `|AD`). The root name is not recorded. Each record adds one to the key's count and sets
its date to the local date (cached for at most 60 s).

Recording is a hash-map update on the DNS thread: no I/O, no await, nothing dropped. At most 262,144 distinct keys are held; further new
names are not recorded (a one-time warning is logged), existing keys keep counting, and loading a file is never truncated. The
`max_entries` argument keeps its reference meaning: when that many records have arrived since the last write, a write is triggered at
once (it is not a size limit).

A generation-enable switch exists per plugin and is always on; the settings layer (M1-f) will drive it, as the reference's
`domain_generation_settings.json` does for the eight production tags.

## Files

Written by periodic writes (every `dump_interval`, or at the `max_entries` trigger; skipped when nothing is recorded), by `/save`, by
`/flush` and when the host stops (no publication then). A write works on a snapshot taken on the DNS thread and runs on the blocking pool;
each file is replaced through a same-directory temporary file and a rename, so a failed write keeps the previous file, leaves no temporary
file and is logged (`/save` and `/flush` answer 500 then). A slow earlier write can never overwrite a later one (snapshots are numbered).

- `file_stat`: `%010d <YYYY-MM-DD> <key>\n`, by count descending (equal counts by key).
- `file_rule`: `full:<name>` for each distinct name (flag suffix removed), in that order.
- `gen_rule`: only when `pattern` is non-empty: the optional `appended_string` line, then `pattern` with `DOMAIN` replaced per distinct name.
  With an empty `pattern` the file is emptied.

The statistics file is read when the host (or a newly installed managed view) starts serving: lines of two fields are `count key` (date =
today), three or more `count date key`, others are skipped, a non-numeric count is 0, the last duplicate wins. Names loaded this way are
not injected into the mapper again. Files written by the reference are read identically and the reference reads native files
(`tools/m1c1-interop`, fixtures `rust/native-host/tests/fixtures/domain_output/`).

## Publication (`domain_set_url`)

After each write except the final one the distinct names are posted as `{"values":["full:<name>",...]}` (empty after `/flush`).

- A URL whose host is `localhost` (when the API listens on loopback or the wildcard), the `api.http` address (any loopback address when the API
  listens on the wildcard), and whose port is the `api.http` port is performed **in-process through the shared URL-action entrance**
  (`local_url`, see [native-requery](native-requery.md)): the same plugin routes the listener serves, with the same admission check and management
  lease (for a `domain_set_light` `/post`: normalisation, atomic file write, rebuild of the dependent mappers); the effect equals the HTTP call.
- Any other URL gets an HTTP/HTTPS `POST` (`Content-Type: application/json`, 5 s, no redirects, no proxy); the response is ignored.
- A local URL naming an unknown provider logs the 404, as the reference's would.

One publication is in flight per plugin; a newer list replaces one still waiting. A failure is logged. The in-process call is never abandoned half way; the HTTP one is bounded by 5 s.

## `target_mapper`

The first record of a key adds the lower-case name to this plugin's **layer** of the mapper's overlay (the layer carries the plugin's fixed
`target_mark` and `target_tag`); the next query for that exact name evaluates as a mapper match: the marks and tags of every layer that holds
the name are added to whatever the rules say (hot values first, as `QuickAdd`), and the mapper's default mark/context mark/tag are not
applied. No matcher is rebuilt for this. The overlay is read at each evaluation and is independent of the matcher generation: a rebuild
neither loses entries nor affects queries in flight. (The reference clears its hot map at each rebuild, so a name recorded since the last
rebuild silently falls back to its default routing; that is not kept.) `/flush` empties the plugin's layer together with its statistics,
and names loaded from the statistics file at start are added to the layer too (so what was learned keeps its routing after a restart or a
managed apply even when the rule file is older; in production the mapper rule for the published list has the same mark and tag). A layer holds
at most the key cap.

## API (`/plugins/<tag>/`, GET only; other methods 405, unknown action 404)

- `show?q=&limit=&offset=`: `text/plain; charset=utf-8`; `q` is a lower-cased substring of the key, `limit` defaults to 100 when invalid or
  `<= 0`, `offset` is clamped at 0; lines `%010d <date> <key>` by count descending; headers `X-Total-Count` (all matches) and
  `Access-Control-Expose-Headers: X-Total-Count`.
- `save`: 200 `domain_output saved` (`text/plain; charset=utf-8`). `flush`: clears memory, writes the empty files, publishes the empty list; 200 `domain_output flushed`.
  While the plugin is retired by a managed apply these answer 503.
- `restartall`: 501 (the host has no self-restart; nothing in the UI uses it).

The capability `lists.remembered` is supported when a `my_*` or `top_domains` plugin exists.

## Lifecycle

Statistics load and the writer starts when the host starts serving. On a managed apply the retired view saves synchronously, stops
recording and never writes again before the new view loads the file (when a publication is still in flight it leaves `file_rule` and
`gen_rule` to that publication, which writes the provider's file; they are regenerated at the new view's next write); at host stop each plugin is closed (writer joined) and writes once more.
