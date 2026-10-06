# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust tool for running apps across your computers. The goal is to tell
Fleet what to run and which computer should take over if the main one goes down.

## What works today

- `run` reads a JSON file, starts the requested copies, and restarts them when
  they exit. Ctrl+C stops its direct child processes.
- `validate` checks a manifest without starting anything.
- `agent` runs a workload and restarts missing copies. `status` shows its name
  and real process counts over TCP. Ctrl+C stops the agent's direct child processes.
- `stop` stops the workload without closing the agent. It sets the desired count
  to zero in memory; restarting the agent reloads the unchanged manifest.
- `scale` changes the copy count while the agent runs, including starting again
  after `stop`. The agent adds or removes copies to match the new target.

Replica counts are limited to 32. Invalid requests return an error.

Use the agent on localhost only: there is no authentication yet. It handles one
client at a time, with idle connection timeouts. Remote deployment, failover,
and data transfer are still planned.

## Try it

Install Rust, then run from this folder:

```sh
cargo run -- run examples/demo.json
```

The demo runs `/bin/sleep` for ten seconds, then starts it again. It works on
macOS and Linux. Edit `examples/demo.json` to change the program or copy count.

To try the agent, run these in separate terminals:

```sh
cargo run -- agent --listen 127.0.0.1:7070 --manifest examples/demo.json
cargo run -- status --address 127.0.0.1:7070
cargo run -- scale --replicas 3 --address 127.0.0.1:7070
cargo run -- stop --address 127.0.0.1:7070
```

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
