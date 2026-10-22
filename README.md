# Homework 5 — `blazing_shell`

A minimal shell in Rust based on lecture 9, “Process Control”. Implements all three parts of the assignment: command execution, error handling, and a single pipe. This is an educational shell, **not** a replacement for Bash or a sandbox for untrusted commands.

## Running

```sh
cargo run
```

Enter commands after the `blazing_shell> ` prompt:

```text
blazing_shell> rustc --version
blazing_shell> cat Cargo.toml | grep name
blazing_shell> exit
```

The second line requires `cat` and `grep` to be installed (for example, on Linux). The `exit` command or end of input terminates the shell.

## How it works

- `src/main.rs`: reads lines and splits on whitespace; launches commands with arguments through `std::process::Command` without `sh -c`; waits for completion and reports errors/nonzero exit statuses to stderr.
- For `left | right`, the first process's stdout is passed to the second process's stdin through `Stdio::piped()` and `Stdio::from(...)`. The parent does not read the entire stream into memory; waiting for both processes releases resources. If the second process fails to start, the first is stopped and waited for.
- Empty lines and invalid pipe syntax do not terminate the shell. Invalid UTF-8 and a nonexistent program produce a clear error followed by a new prompt.
- `tests/shell_tests.rs` and the test program `tests/support/shell_fixture.rs` verify command execution, errors, pipes, and a data stream larger than the pipe's capacity without depending on Unix utilities.

## Verification

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Assignment boundaries

At most one `|` is supported. Arguments are separated by whitespace; quoting, escaping, substitution, file redirection, background jobs, and built-in commands other than `exit` are not implemented. `|` is recognized even without surrounding spaces. Program execution follows the OS executable lookup rules. The shell itself does not invoke `sh -c` or process other metacharacters; special executable types (such as `.bat` on Windows) may be launched by the OS through an interpreter. Input is intended for a local educational session, not an unbounded network stream.
