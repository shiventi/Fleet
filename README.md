# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust CLI to run and check apps across computers. Each agent manages
one app with up to 32 copies, restarts missing copies, and accepts remote stop
and scale commands.

Failover, file transfer, and running several apps under one agent are not ready yet.

## Try It

Install Rust, then run from this folder on macOS or Linux:

```sh
cargo run -- run examples/demo.json
cargo run -- validate examples/demo.json
```

The demo starts `sleep` again every ten seconds. Ctrl+C stops its direct children.

## Remote Apps

Follow the [setup guide](docs/usage.md#device-identity) to approve device keys
and share a token. Then use `agent-device`, `status-device`, `stop-device`,
and `scale-device` to run and control an app.

For a live view of several agents:

```sh
cargo run -- dashboard-device --manifest examples/cluster-device.json
```

Edit the cluster file with your agents' addresses and approved peer names first.
Up/Down selects a node; `=` or `+` adds a copy; `-` removes one.
`q`, Esc, or Ctrl+C closes the view without stopping agents.
`watch-device` prints counts instead. Use `cargo run -- --help` for all commands.

## Limits

Remote commands need mutual TLS and a shared token. Approved clients have full
app control. Keep secrets out of Git and use a private network with a firewall.

Scale changes stay in memory. Apps are not sandboxed, and cleanup stops direct
children only. Process counts are not health checks; an unreachable agent does
not prove its app stopped. See [usage notes](docs/usage.md#limits) for details.

## Docs and Tests

[Setup](docs/usage.md) · [Rustdocs](https://shiventi.github.io/Fleet/fleet/)

```sh
cargo test --locked
cargo build --locked
```

Code notes live in `docs/*.txt`, outside the Rust files.
