# Fleet

**Work in progress. Not ready for production.**

Fleet is a Rust tool for running apps across your computers. The goal is to tell
Fleet what to run and which computer should take over if the main one goes down.

## What works today

So far, it works on one computer. It reads a JSON file, starts the number of
copies you ask for, and restarts them when they exit. Ctrl+C stops the programs
it started and exits Fleet.

Running apps on other computers, switching to a backup, and moving app data
are not built yet. Neither is stopping extra copies when fewer are needed.

## Try it

Install Rust, then run from this folder:

```sh
cargo run -- run examples/demo.json
```

The demo runs `/bin/sleep` for ten seconds, then starts it again. It works on
macOS and Linux. Edit `examples/demo.json` to change the program or copy count.

## Checks

```sh
cargo run -- validate examples/demo.json
cargo test
```
