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

Agent commands require a shared `FLEET_TOKEN`. Wrong tokens are rejected without
changing the workload. Missing or blank tokens prevent the agent or client from
starting. `run` and `validate` do not need a token.

Keep the agent on localhost. Tokens do not encrypt TCP traffic; use an SSH tunnel
for remote connections. The agent handles one client at a time, with idle
connection timeouts. Remote deployment, failover, and data transfer are still planned.

## Try it

Install Rust, then run from this folder:

```sh
cargo run -- run examples/demo.json
```

The demo runs `/bin/sleep` for ten seconds, then starts it again. It works on
macOS and Linux. Edit `examples/demo.json` to change the program or copy count.

### Create a token

Run this once. It creates a random token in a private file outside the repo:

```sh
mkdir -p "$HOME/.config/fleet"
(umask 077; openssl rand -hex 32 > "$HOME/.config/fleet/token")
chmod 600 "$HOME/.config/fleet/token"
```

Keep this file private. Do not commit tokens, paste them into logs, or share them.
Creating a new token replaces the old one; restart the agent with the new token.

### Try the agent

In the first terminal:

```sh
export FLEET_TOKEN="$(cat "$HOME/.config/fleet/token")"
cargo run -- agent --listen 127.0.0.1:7070 --manifest examples/demo.json
```

In a second terminal, load the same token and send commands:

```sh
export FLEET_TOKEN="$(cat "$HOME/.config/fleet/token")"
cargo run -- status --address 127.0.0.1:7070
cargo run -- scale --replicas 3 --address 127.0.0.1:7070
cargo run -- stop --address 127.0.0.1:7070
```

The token file is not loaded automatically; `export` supplies it to Fleet.

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
