# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust CLI to run and check apps across computers. Each agent runs
one or more apps with up to 32 copies each and restarts missing copies.
The dashboard can select and scale each app, and logs can target an app by name.
Status, stop, scale, and watch commands still require a single-app agent.

Failover and file transfer are not ready yet.

## Try It

Install Rust, then run from this folder on macOS or Linux:

```sh
cargo run -- run examples/demo.json
cargo run -- validate examples/demo.json
```

The demo starts `sleep` again every ten seconds. Ctrl+C stops its process groups.
Both agent modes also accept [examples/apps.json](examples/apps.json) after setup.
`run` and `validate` still take single-app files.

## Remote Apps

Follow the [setup guide](docs/usage.md#device-identity) to approve device keys
and share a token. Then use `agent-device`, `status-device`, `stop-device`,
and `scale-device` to run and control an app.

For a live view of several agents:

```sh
cargo run -- dashboard-device --manifest examples/cluster-device.json
```

Edit the cluster file with your agents' addresses and approved peer names first.
Tab switches between Agents and Apps; arrows select a row.
In Apps, `=` or `+` adds a copy; `-` removes one. Press `l` for the selected app's
live logs; Esc returns to Apps. Up/Down scrolls logs; End follows new lines.
Esc returns to Agents, or exits there; `q` or Ctrl+C exits without stopping agents.
`watch-device` prints counts instead. Use `cargo run -- --help` for all commands.
The dashboard shows the selected host's CPU and RAM, not per-app usage.
Use `cargo run -- metrics` for local CPU and RAM readings.

Follow an app's stdout and stderr with `logs-device --address IP:PORT --peer NAME --app APP --follow`.
See the [log demo](docs/usage.md#logs) for a local test.

## Limits

Remote commands need mutual TLS and a shared token. Approved clients have full
app control. Keep secrets out of Git and use a private network with a firewall.

Scale changes stay in memory. Apps are not sandboxed; processes that leave their
app's process group can escape cleanup. Counts are not health checks; an unreachable
agent does not prove its app stopped. See [usage notes](docs/usage.md#limits) for details.

## Docs and Tests

[Setup](docs/usage.md) · [Rustdocs](https://shiventi.github.io/Fleet/fleet/)

```sh
cargo test --locked
cargo build --locked
```

Code notes live in `docs/*.txt`, outside the Rust files.
