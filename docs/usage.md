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

## Certificates Instead

After loading the shared token, create 30-day test certificates:

```sh
sh scripts/dev-certs.sh
```

Set `TLS="$HOME/.config/fleet/dev-tls"` in both terminals. On the agent:

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

- One app per agent, 0-32 copies. Stop keeps the agent alive; scale above zero
  starts copies again. Targets reset to the JSON file when the agent restarts.
- Approved clients with the token have full control. Restart agents and clients
  after key or token changes; existing connections are not revoked.
- Keep secrets out of Git. Apps do not inherit the token, but are not sandboxed.
  Cleanup stops direct children, not their full process trees.
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
