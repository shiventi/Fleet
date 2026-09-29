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

Use the agent on localhost only: there is no authentication yet. It handles one
client at a time, with idle connection timeouts. Remote deployment, failover, data transfer, and stopping extra copies
are still planned.

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
```

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
