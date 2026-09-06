# Fleet

Fleet is a Rust project for managing applications across a few Linux machines.
The goal is to describe what should run, where it should run, and how it should
recover when a process or machine fails.

For example, you could keep an application on a home server with a VPS ready as
its backup. Fleet aims to handle placement, health checks, and controlled
handoffs, with explicit configuration for application state.

## What works today

Fleet currently runs on one machine. It reads a JSON manifest, starts the
requested number of processes, checks them every second, and starts replacements
when they exit. Finished process handles are removed during each check.

Remote deployment, primary/standby recovery, and state transfer are planned.
Fleet is in early development and is not ready to manage production services.

## Try it

With a current stable Rust toolchain installed, run from the project directory:

```sh
cargo run -- run examples/demo.json
```

The demo manifest contains:

```json
{
  "name": "demo-service",
  "program": "/bin/sleep",
  "args": ["10"],
  "replicas": 1
}
```

This starts one process that waits ten seconds and exits. Fleet then starts a
replacement. Change `replicas` before launching Fleet to try multiple copies.
The demo works on macOS and Linux, where `/bin/sleep` is available.

Press Ctrl+C to exit. Fleet does not yet implement graceful child-process cleanup;
the demo's sleep processes finish naturally after ten seconds.

## How it works

- A workload specification describes the executable, arguments, and desired count.
- A process runner launches processes and checks which are still alive.
- A planner compares the running count with the requested count.
- A loop carries out start decisions and repeats the check.

The manifest is read once at startup. Stop actions are currently reported but not
executed, and file, configuration, or process errors can terminate Fleet.

## Development

```sh
cargo test
cargo doc --no-deps --document-private-items --open
```

The current tests cover the planner's start and stop decisions.
