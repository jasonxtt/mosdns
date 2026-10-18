# Go reference toolchain

The `rust` branch has no Go source. Differential tests against the Go MosDNS (E2E-1,
5D, the M1 oracles and interop tools) use binaries built from a **pinned commit** of the
Go repository (`jasonxtt/mosdns` `main`, kept at `/Users/tom/github/mosdns`).

```sh
# Go 1.26+ on the build machine (the Mac); cross-compile for the Linux test VM:
GOOS=linux GOARCH=amd64 tools/go-reference/build.sh /tmp/go-ref            # all targets
GOOS=linux GOARCH=amd64 tools/go-reference/build.sh /tmp/go-ref mosdns     # the reference program
```

- `PINNED_COMMIT` is the Go commit every tool is verified against (v0.7.3 = `75f26677`).
  `MOSDNS_GO_COMMIT` overrides it, `MOSDNS_GO_REPO` moves the repository.
- The Go repository is only read (`git archive`); nothing in it is modified.
- `mosdns` is what `tools/e2e-1` and `tools/5d-*` take as `--go`. The other targets are the
  thin wrappers kept here as `tools/<name>/main.go` (they import the Go packages, so they
  can only compile inside the Go tree; the script overlays them there).
- The test VM's Go may be older than the pinned `go.mod` asks for: build on the Mac and copy
  the binaries over.
- The Rust build and `cargo test` never use any of this.
