# Setup

Run from the repo root on macOS or Linux.

## Device identity

On each computer:

```sh
cargo run -- identity > this-device.pub
cargo run -- identity --fingerprint
```

Transfer only the public file to the other computer as `other-device.pub`.
Compare the full fingerprint through a trusted channel, then approve it on both:

```sh
cargo run -- peers add other-device other-device.pub
```

Use `peers list`, `peers show other-device`, or `peers remove other-device`
to manage approvals. Each computer needs its own private key. Keep
`~/.config/fleet/identity/` private and backed up; never copy private keys
between devices. Keys have no expiry. Windows key storage and automatic pairing
are not built yet.

## Token

Create one shared token with OpenSSL. This refuses to overwrite an existing file:

```sh
mkdir -p "$HOME/.config/fleet"
(umask 077; set -C; openssl rand -hex 32 > "$HOME/.config/fleet/token")
chmod 600 "$HOME/.config/fleet/token"
```

Copy it securely to the same path on the other computer, with mode `600`.
In every agent and client terminal:

```sh
export FLEET_TOKEN="$(cat "$HOME/.config/fleet/token")"
```

## Run and Control

Agent terminal:

```sh
cargo run -- agent-device --listen 127.0.0.1:7070 --manifest examples/demo.json
```

Use `examples/apps.json` to run two apps and select them in the dashboard.
The commands below and watch still require a single-app agent. `run` and `validate` still
take single-app files.

Client terminal:

```sh
cargo run -- status-device --address 127.0.0.1:7070 --peer other-device
cargo run -- scale-device --address 127.0.0.1:7070 --peer other-device --replicas 2
cargo run -- stop-device --address 127.0.0.1:7070 --peer other-device
```

These IPs are for local tests. Between computers, replace them with the agent's
reachable private IP and add `--allow-remote` to the agent. Restrict its port
with a firewall; wildcard IPs are rejected.

For watch or dashboard, edit `examples/cluster-device.json`: set each address
and approved `peer`, keeping `server_name` as `localhost`.
Run `watch-device` or `dashboard-device` with `--manifest` pointing to that file.
In the dashboard, Tab switches Agents/Apps and arrows select rows. In Apps,
use + or = to add a copy and - to remove one. Esc returns to Agents; q exits.
Press l on a selected app to open its logs; Esc returns to Apps.
Up/Down and PageUp/PageDown scroll logs; Home shows the oldest lines and End
follows the latest. Stderr text is red. Old logs stay visible on a fetch error
while the viewer retries; app controls are disabled in this view.
Wide terminals show both tables; narrow terminals show the focused table.
The dashboard's CPU/RAM meters show the selected host, not individual apps.
For this computer's readings only, run `cargo run -- metrics`.

## Logs

After device setup, run `logs-device --address IP:PORT --peer NAME --app APP --follow`.
Use `logs` with the client TLS flags below instead for certificate mode.
Without `--follow` (or `-f`), the command prints one batch and exits.
`--after ID` starts after a printed log ID. Follow drains batches, then checks
every half second when idle. Ctrl+C stops the viewer, not the app or agent;
an in-flight request can take up to five seconds to end.

For a local demo with your test certificates, use two terminals.
In both, set `export FLEET_TOKEN=fleet-local-demo` and
`TLS="$HOME/.config/fleet/dev-tls"`. This token is for local demos only.
Create the test certificates with `sh scripts/dev-certs.sh` first if needed;
the script refuses to overwrite existing files.

Agent terminal:

```sh
cargo run -- agent --listen 127.0.0.1:7080 --manifest examples/logs.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/server.crt" --private-key "$TLS/server.key"
```

Viewer terminal:

```sh
cargo run -- logs --address 127.0.0.1:7080 --app log-demo --follow \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

The app prints a tick each second and a demo warning on stderr every third tick.
To see it in the dashboard, use the same client TLS flags with
`dashboard --manifest examples/cluster-logs.json`, then press Tab and l.
Logs show IDs, process IDs, and stream labels. Terminal control characters are
escaped rather than executed. Unknown app names and wrong tokens return errors.
Each app keeps only its latest 200 lines in memory, capped at 4096 bytes per line.
Replies over 16 KiB are split into batches; an oversized single line is shortened
in the reply with `[truncated]`. Apps may buffer output before Fleet sees it.
CLI follow exits on connection errors; the dashboard retries in the background.
IDs reset after agent restart: reopen the dashboard log view or start the CLI
again without `--after`. There is no disk history. Long lines are clipped to
the dashboard's width; use the CLI to read them in full.

## Certificates Instead

After loading the shared token, create 30-day test certificates:

```sh
sh scripts/dev-certs.sh
```

Set `TLS="$HOME/.config/fleet/dev-tls"` in every terminal tab; shell variables
are not shared between tabs. On the agent:

```sh
cargo run -- agent --listen 127.0.0.1:7070 --manifest examples/demo.json \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/server.crt" --private-key "$TLS/server.key"
```

On the client:

```sh
cargo run -- status --address 127.0.0.1:7070 --server-name localhost \
  --ca-cert "$TLS/ca.crt" --cert "$TLS/client.crt" --private-key "$TLS/client.key"
```

Use the same client TLS flags for `stop`, `scale --replicas 2`, `watch`, and
`dashboard`; views also need `--manifest examples/cluster.json`.
For a remote test cert, pass a new folder and server name to `dev-certs.sh`.
Keep CA and client private keys off the agent. Demo certs are not for production;
renewal and revocation are not built yet.

## Limits

- Several apps per agent, 0-32 copies each. The dashboard controls named apps,
  and log commands select an app by name. Status, stop, scale, and watch require one app.
  Stop keeps the agent alive; scale above zero
  starts copies again. Targets reset to the JSON file when the agent restarts.
- Approved clients with the token have full control. Restart agents and clients
  after key or token changes; existing connections are not revoked.
- Keep secrets out of Git. Apps do not inherit the token, but are not sandboxed.
  Cleanup stops app process groups; descendants that leave the group can escape it.
- Counts are not health checks. A failed connection does not prove an app stopped.
- Failed launches retry after five seconds; apps that start then exit do not wait.
  Each connection has a five-second deadline and 16 KiB message limit.
  The agent serves one client at a time.

## Checks

```sh
cargo test --locked
cargo doc --locked --no-deps --document-private-items
```

Tests use temporary keys. With `expect` installed, also run the terminal test:

```sh
cargo test --locked --test device_connection -- --ignored
```
