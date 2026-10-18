# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust tool for running apps across your computers. The goal is to tell
Fleet what to run and which computer should take over if the main one goes down.
Each agent currently manages one workload, with up to 32 running copies.

## What works today

- `run` reads a JSON file, starts the requested copies, and restarts them when
  they exit. Ctrl+C stops its direct child processes.
- `validate` checks a manifest without starting anything.
- `identity` creates or reuses a saved device key and prints only its public key
  on macOS/Linux. `--fingerprint` prints its SHA-256 fingerprint instead.
  Automatic pairing is not built yet.
- `peers` lists, shows, adds, and removes saved device public keys.
  Device-key commands use this list to check remote identities.
- `agent` runs a workload and restarts missing copies. `status` shows its name
  and real process counts over mutual TLS. Ctrl+C stops the agent's direct children.
- `agent-device`, `status-device`, `stop-device`, `scale-device`, `watch-device`,
  and `dashboard-device` use saved device keys instead of certificate files.
  Both sides must approve each other's keys and share a token.
- `stop` stops the workload without closing the agent. It sets the desired count
  to zero in memory; restarting the agent reloads the unchanged manifest.
- `scale` changes the copy count while the agent runs, including starting again
  after `stop`. The agent adds or removes copies to match the new target.
- `watch` checks agents listed in a cluster JSON file and prints their counts or
  errors. It keeps checking even when one agent cannot be reached.
- `dashboard` shows live process counts and connection errors in a terminal
  table. Select a node with Up/Down; `=` adds a copy and `-` removes one.
  Background checks and commands keep the screen responsive.

Replica counts are limited to 32. Invalid requests return an error.
Failed launches are reported instead of crashing Fleet. Both `run` and `agent`
wait at least five seconds before retrying a failed launch. This delay does not
apply to apps that start successfully and then exit.

Agent commands require mutual TLS and a shared `FLEET_TOKEN`. Certificate mode
checks the server's name; device-key mode checks the selected peer's public key.
TLS encrypts commands and tokens; plaintext connections are rejected.
Missing or blank tokens prevent startup, and workloads
do not inherit the token. `run` and `validate` need neither TLS files nor a token.

Loopback addresses need no remote flag. `--allow-remote` permits a specific remote
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

### Device identity

```sh
cargo run -- identity
cargo run -- identity --fingerprint
```

No token, certificate files, or OpenSSL command is needed for this command.
It creates `~/.config/fleet/identity/device-key.pem` on first use and reuses it on
later runs. Run it twice to confirm that the public key stays the same.
The identity directory is private (`700`), and the key is owner-only
(`600`). Only the public key or its fingerprint is printed. New identities are
fully written before being published without replacing an existing key. Unsafe directories, symlinks,
damaged keys, and broad permissions return errors instead of being repaired silently.

Fingerprints use `SHA256:` followed by 64 lowercase hex characters. They hash the
decoded public key, not PEM text, so line endings do not change the value.
Compare the full fingerprint through a trusted channel before approving a device;
a matching fingerprint alone does not prove who owns the key.

The key has no certificate expiry date. Keep it private and include it in protected
backups; deleting it creates a different identity on next use. Never share it with
another device. The device-key commands use these identities with TLS 1.3
and a shared token. Certificate-based commands remain available too.
Windows identity storage and automatic pairing come later.

### Approved peers

On the other computer, export its public key with `fleet identity > pi-public.pem`
(or `cargo run -- identity > pi-public.pem`). Transfer only this public file, and
verify it belongs to the intended device before approving it:

```sh
cargo run -- peers add pi pi-public.pem
cargo run -- peers list
cargo run -- peers show pi
cargo run -- peers remove pi
```

The list is saved privately in `~/.config/fleet/identity/trusted-peers.json`.
Commands check Fleet's P-256 public keys, reject private keys and duplicates, and
lock the list while updating it. Keep the separate `trusted-peers.lock` file in
place. Names use 1-64 ASCII letters, digits, hyphens, underscores, or dots.
Public-key files are limited to 16 KiB and the saved list to 1 MiB. Peer commands
work on macOS/Linux without certificates or a token.
`show` prints one peer's name, fingerprint, and public key without changing the
list; missing names return an error. Its fingerprint should match the other
device's `identity --fingerprint` output.
In device-key mode, the agent accepts any client key on its approved list; the
client accepts only the named server's key. Both still require the shared token.
An approved client with the token can control the agent's workload; there are no
read-only roles or per-command permissions yet.
Restart the agent after changing its approvals. Existing connections are not
revoked. Peer changes do not affect certificate-based access.

### Try device-key connections

Export each computer's public key with `identity` and approve it on the other
computer using `peers add`. Compare fingerprints through a trusted channel first.
Use a separate identity on each computer; never copy private keys between them.
Set the same `FLEET_TOKEN` in both terminals, then run:

```sh
# On the agent computer:
cargo run -- agent-device --listen 127.0.0.1:7070 --manifest examples/demo.json

# On the client, using the saved name for the agent's key:
cargo run -- status-device --address 127.0.0.1:7070 --peer my-server
cargo run -- scale-device --address 127.0.0.1:7070 --peer my-server --replicas 2
cargo run -- stop-device --address 127.0.0.1:7070 --peer my-server
```

These addresses are for local testing. Between computers, use the agent's specific
reachable IP and add `--allow-remote` to the agent. Restrict access with a firewall.
Stopping keeps the agent running; scale above zero to start the workload again.
Counts may take another agent loop to reach a new target.

For multiple agents, edit `examples/cluster-device.json` with their addresses and
saved peer names, then use:

```sh
cargo run -- watch-device --manifest examples/cluster-device.json
cargo run -- dashboard-device --manifest examples/cluster-device.json
```

Each node's `peer` selects its approved key; `name` is just its display label.
The example peer names `local-one` and `local-two` must already be approved.
Keep `server_name` as `localhost` in device-key manifests; this mode checks the
selected key, not the hostname. Missing or unknown peers prevent startup, and
wrong keys fail the connection. Clients load credentials once, in manifest order.

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

### Try the dashboard

With the agent running, use the client terminal from above:

```sh
cargo run -- dashboard --manifest examples/cluster.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

The screen shows agents listed in your cluster manifest, which can run on different
computers. It does not discover devices or list saved peers automatically.
Each row shows the latest process counts or connection error. The example has
two agents; start another on port `7071` to connect both. Stop and restart an
agent to see its row update automatically. A failed check does not prove the app stopped.

Use Up/Down to select a node, then `=` to add a copy or `-` to remove one.
No Ctrl or Shift is needed; `+` also works. Counts stay between 0 and 32;
zero stops the workload, and `=` starts it again.
Ctrl-minus also removes a copy if your terminal passes that shortcut to Fleet.
Only one scale command runs at a time. Results appear below the table, and the
running count may take another check to catch up. Scaling changes the agent's
in-memory target, not its manifest. After a command fails, Fleet waits for a fresh
successful check before allowing another; the agent might have applied it even
if its reply was lost.

One background thread checks nodes in order, with a two-second pause after each
round. A slow node delays later checks, not keyboard input. Credentials are loaded
once; restart the dashboard after changing them. Addresses are hidden on narrow
screens, and long errors may be clipped. Exit with `q`, Esc, or Ctrl+C; quitting
waits for the current check or pause to finish and does not stop agents.

### Watch agents

Start a second agent using the agent command above, changing its port to `7071`.
Both test agents use the same demo credentials. In the client terminal, run:

```sh
cargo run -- watch --manifest examples/cluster.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

Checks run in batches of up to four, with a two-second pause after each round.
Before polling, Fleet checks the cluster entries, token, and local TLS files.
The watcher loads credentials once and shares them across checks. Each check
opens a new connection; restart the watcher after changing credentials.
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
cargo test --locked
cargo build --locked
```

Identity and peer tests use temporary directories, not your saved device keys.
Run `cargo test --locked device_tls::tests` for the mutual device-key checks and
in-memory handshakes; these open no network ports and change no saved identities.
Run `cargo test --locked --test device_connection` for the real localhost test.
It uses temporary identities and a temporary port, checks status and rejected
keys/tokens, and verifies Ctrl+C stops the agent and its children.

With `expect` installed, run the optional real-terminal test:

```sh
cargo test --locked --test device_connection device_dashboard_and_watch_use_the_right_key_for_each_agent -- --ignored
```

It starts two agents with different keys, drives dashboard selection and scaling,
checks wrong-key rejection and watch output, and cleans up the temporary agents.
`expect` is only needed for this test, not for running Fleet.
