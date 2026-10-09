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
- `watch` checks agents listed in a cluster JSON file and prints their counts or
  errors. It keeps checking even when one agent cannot be reached.

Replica counts are limited to 32. Invalid requests return an error.

Agent commands require trusted client/server certificates and a shared `FLEET_TOKEN`.
TLS encrypts commands and tokens; plaintext connections are rejected. The client
checks the server's name. Missing or blank tokens prevent startup, and workloads
do not inherit the token. `run` and `validate` need neither TLS files nor a token.

The agent uses localhost by default. `--allow-remote` permits a specific remote
IP; use a private network such as Tailscale, with a firewall. Wildcard addresses
are rejected. It handles one client at a time, with a five-second connection
deadline and 16 KiB message limit. Failover and data transfer are still planned.

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
testing. Only approved controllers should receive certificates from this CA;
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

### Watch agents

Start a second agent using the agent command above, changing its port to `7071`.
Both test agents use the same demo credentials. In the client terminal, run:

```sh
cargo run -- watch --manifest examples/cluster.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

Checks run in batches of up to four, with a two-second pause after each round.
Before polling, Fleet checks the cluster entries, token, and local TLS files.
Each node needs a name, certificate name, and numeric IP with a nonzero port;
wildcard IPs are not allowed. Agents still verify credentials when connecting.
Results print as checks finish. A slow check delays the next batch, not other
checks in its batch. Ctrl+C stops only the watcher. It does not move workloads;
a failed check does not prove that an app stopped.

### Test across computers

Create a separate demo identity with the remote agent's certificate name:

```sh
sh scripts/dev-certs.sh "$HOME/.config/fleet/ovh-demo" ovh-sparq
```

Build Fleet on the Linux server. Copy only `ca.crt`, `server.crt`, `server.key`,
and the shared token to it, keeping private files owner-only. Keep `ca.key` and
`client.key` on your Mac. Bind the agent to its Tailscale IP with `--allow-remote`.
On the client, use that IP with `--address` and `ovh-sparq` with `--server-name`,
plus the new client certificate paths. The name verifies identity; it need not
resolve in DNS. Allow the agent's port only from your controller on Tailscale.

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
