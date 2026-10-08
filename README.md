# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust tool for running apps across your computers. The goal is to tell
Fleet what to run and which computer should take over if the main one goes down.

## What works today

- `run` reads a JSON file, starts the requested copies, and restarts them when
  they exit. Ctrl+C stops its direct child processes.
- `validate` checks a manifest without starting anything.
- `agent` runs a workload and restarts missing copies. `status` shows its name
  and real process counts over mutual TLS. Ctrl+C stops the agent's direct children.
- `stop` stops the workload without closing the agent. It sets the desired count
  to zero in memory; restarting the agent reloads the unchanged manifest.
- `scale` changes the copy count while the agent runs, including starting again
  after `stop`. The agent adds or removes copies to match the new target.

Replica counts are limited to 32. Invalid requests return an error.

Agent commands require trusted client/server certificates and a shared `FLEET_TOKEN`.
TLS encrypts commands and tokens; plaintext connections are rejected. The client
checks the server's name. Missing or blank tokens prevent startup, and workloads
do not inherit the token. `run` and `validate` need neither TLS files nor a token.

This prototype listens on localhost only. It handles one client at a time, with
a five-second connection deadline and 16 KiB message limit. Remote deployment,
failover, and data transfer are still planned.

## Try it

Install Rust, then run from this folder:

```sh
cargo run -- run examples/demo.json
```

The demo runs `/bin/sleep` for ten seconds, then starts it again. It works on
macOS and Linux. Edit `examples/demo.json` to change the program or copy count.

### Set up local credentials

Requires OpenSSL. Run once; credentials stay outside the repo:

```sh
mkdir -p "$HOME/.config/fleet"
(umask 077; set -C; openssl rand -hex 32 > "$HOME/.config/fleet/token")
chmod 600 "$HOME/.config/fleet/token"
sh scripts/dev-certs.sh
```

Keep tokens, private keys, and the demo CA key private. The commands refuse to
overwrite an existing token or certificate directory. Certificates expire after 30 days and are only for
local testing. Only approved controllers should receive certificates from this CA;
trusted clients with the token have full control. Renewal and revocation are not built yet.

### Try the agent

In the first terminal:

```sh
export FLEET_TOKEN="$(cat "$HOME/.config/fleet/token")"
TLS="$HOME/.config/fleet/dev-tls"
cargo run -- agent --listen 127.0.0.1:7070 --manifest examples/demo.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/server.crt" --private-key "$TLS/server.key"
```

In a second terminal, load the same token and send commands:

```sh
export FLEET_TOKEN="$(cat "$HOME/.config/fleet/token")"
TLS="$HOME/.config/fleet/dev-tls"
cargo run -- status --address 127.0.0.1:7070 --server-name localhost \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

Use the same client options with `scale --replicas 3` or `stop` in place of `status`.
The token file is not loaded automatically; `export` supplies it to Fleet.
Workloads are not sandboxed; untrusted apps need separate OS users or containers.
Commit the setup script, not generated credentials or certificates.

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
