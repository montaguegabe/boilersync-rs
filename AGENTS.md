# Boilersync

`boilersync` is a Rust command-line tool built with clap (derive API).

## Conventions

- Build with `cargo build --release`; the binary is `target/release/boilersync`. Run `cargo clippy --all-targets` and `cargo fmt` before committing.
- Keep `main.rs` for argument parsing and wiring. Put logic in modules once a file passes a few hundred lines.
- Exit codes: 0 success, 1 the tool ran but found a problem or could not finish, 2 usage error. Print errors to stderr prefixed with the tool name.
- Never follow symlinks when walking directories unless the user asks for it.
- Anything that removes files sends them to the Trash (the `trash` crate) rather than deleting them, and needs an explicit confirmation or flag.
- Do not hard-code machine-specific paths. Derive from `$HOME`, arguments or environment variables.

## Testing

- Unit-test pure helpers in the same file under `#[cfg(test)]`.
- For interactive behaviour, drive the binary through a pty (for example a short Python script with `pty.fork`) rather than mocking the terminal.
