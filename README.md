# leetui

A lightweight, local-first LeetCode CLI and terminal interface written in Rust.

`leetui` talks directly to LeetCode's web GraphQL and judge endpoints. It does not use a
third-party LeetCode service or SDK. Problem statements, editorials, metadata, and images are
cached locally; running and submitting code still requires a network connection to LeetCode.

> LeetCode does not publish a supported API. Its web endpoints can change without notice.

## Current features

- Browse and search all free problems in a Ratatui interface.
- Read formatted statements and free official editorials.
- Display statement images using Kitty/Sixel/iTerm2 when supported, with a text fallback.
- Generate starter files for any language returned by LeetCode.
- Use `$VISUAL`, `$EDITOR`, or a configured editor/IDE.
- Run sample or custom input with LeetCode's Run Code judge.
- Submit solutions and receive verdict, runtime, memory, output, and errors.
- Browse personal submission history and source code.
- Continue browsing cached content offline.

Premium problems are hidden by default, and `leetui` does not attempt to bypass locked content.

## Build

```sh
cargo build --release
```

The binary is `target/release/leetui`. Rust 1.90 or newer is required. Rustls is used for HTTPS,
so OpenSSL is not required.

## Install from source

From the project directory, install `leetui` for the current user using the dependency versions
recorded in `Cargo.lock`:

```sh
cargo install --path . --locked
```

Cargo installs the binary to `~/.cargo/bin/leetui`. Rustup normally adds this directory to
`PATH`. If `leetui` is not found, add it to your shell configuration:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

After updating the source, reinstall the binary with:

```sh
cargo install --path . --locked --force
```

To remove it:

```sh
cargo uninstall leetui
```

Do not run `cargo install` with `sudo`; user-local installation does not require elevated
permissions.

## Authentication

Problem discovery and statements work anonymously. Run, submit, personal status, and submission
history require the `LEETCODE_SESSION` and `csrftoken` cookies from an authenticated browser.

1. Sign in at <https://leetcode.com>.
2. Open browser developer tools.
3. Find cookies for `https://leetcode.com`.
4. Run `leetui login` and paste both values into the hidden prompts.

The credentials file is created with mode `0600` on Unix. Treat it like a password. Environment
variables `LEETCODE_SESSION` and `LEETCODE_CSRF_TOKEN` override the saved values.

Avoid passing cookies with command-line flags because shell history and process listings may
expose them.

## Quick start

```sh
leetui login
leetui sync
leetui
```

Useful commands:

```sh
leetui list --difficulty medium --tag array
leetui show two-sum
leetui editorial two-sum
leetui pick two-sum --lang rust
leetui edit two-sum
leetui run two-sum --input $'[2,7,11,15]\n9'
leetui run two-sum --input-file cases.txt
leetui run two-sum --examples
leetui submit two-sum
leetui submissions two-sum
leetui submissions --show SUBMISSION_ID
```

Numeric problem IDs and LeetCode problem URLs are also accepted.

## Configuration

```sh
leetui config language rust
leetui config workspace '~/leetcode'
leetui config editor 'nvim {file}'
leetui config editor 'code --wait {dir}'
leetui config
```

If no editor command is configured, `$VISUAL` and then `$EDITOR` are used. `{file}` and `{dir}`
are replaced without invoking a shell.

Default paths follow the platform's standard directories. On Linux:

```text
~/.config/leetui/config.toml
~/.local/state/leetui/credentials.toml
~/.cache/leetui/
~/leetcode/
```

Each problem workspace contains:

```text
0001-two-sum/
├── problem.md
├── solution.rs
└── .leetui.toml
```

The selected language in `.leetui.toml` is authoritative when reopening a workspace. It changes
only when `--lang` is explicitly supplied. `problem.md` is generated only once, so local notes
are preserved.

## TUI keys

| Key | Action |
|---|---|
| `/` | Search |
| `j` / `k` | Move or scroll |
| `Enter` | Open problem |
| `Tab` | Load official editorial |
| `i` | Preview the first statement image |
| `e` | Open solution in configured editor |
| `r` | Run the sample testcase |
| `S` | Confirm and submit the current solution |
| `m` | Show personal submissions |
| `Esc` | Close image or go back |
| `q` | Back or quit |

Use the CLI `run --input` or `run --input-file` commands for custom multiline testcases.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo audit
```

Authenticated judge calls are deliberately not part of the automated test suite.
