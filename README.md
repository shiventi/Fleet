# Fleet

Fleet is a Rust tool being built to run apps across multiple computers.
The goal is simple: describe what you want running, and let Fleet keep it running,
with another computer ready to take over if needed.

## What works today

For now, Fleet runs on one computer. It reads a JSON file, starts the requested
number of copies of a program, and restarts them when they exit.

Running apps on other computers, switching to backups, and moving data are
planned. Fleet is not ready for production use.

## Try it

Install Rust, then run from this folder:

```sh
cargo run -- run examples/demo.json
```

The demo runs `/bin/sleep` for ten seconds, then starts it again. It works on
macOS and Linux. Edit `examples/demo.json` to change the program or copy count.

Press Ctrl+C to exit. Automatic cleanup on exit is not connected yet, and stopping
extra copies is not implemented.

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
