# UDP truncation validation

`main.go` is a validation-only oracle pinned to miekg/dns v1.1.72. It reads
JSON lines `{name, wire: hex, size}`, calls `Msg.Truncate`, then returns packed
wire, length, TC and section counts. It performs no network I/O. It is not a
runtime Go dependency. Build only in the authorized VM, using cached Go and
dependencies, `GOPROXY=off GOSUMDB=off GOTOOLCHAIN=local`, `go build -p=2`.

The Rust `udp_response` unit tests optionally invoke it using
`MOSDNS_UDP_TRUNCATION_ORACLE=/absolute/go-truncate` and append numerical rows
to `MOSDNS_UDP_TRUNCATION_REPORT=/private/oracle.jsonl`. Use a fresh report
path and `--test-threads=1` for deterministic evidence. Section counts and TC
must agree; byte equality is reported separately because an already-fitting
Rust response preserves its existing compressed encoding and a signed
response preserves its exact original bytes.

`replay.py` runs an actual candidate binary with a controlled loopback upstream
inside a task-owned, **loopback-only Linux network namespace**. It reads a
private JSON array of saved `{token, query_wire, rust_wire, go_wire}` rows.
The complete old Rust response is returned by the mock with the actual
upstream transaction ID, then the candidate applies its downstream budget.
The query payload is unchanged. This tests the UDP response boundary, not
the original routing configuration or original client identity.

The script compares candidate size/TC/section counts with both the saved Go
reply and the oracle, checks final native audit TC/answer count, stops its
process/thread and saves raw details only in the supplied fresh work directory.
Keep namespace creation/deletion under the outer controller's ownership;
delete it only after every owned process exits.

Pass `--binary`, `--oracle`, `--samples` and `--work` absolute paths. All raw
samples, output and host logs must stay in a 0700 directory **outside this
repository**. Do not copy production names/addresses/credentials into fixtures,
documentation or commands committed here. This directory contains no private
corpus or expected production addresses.
