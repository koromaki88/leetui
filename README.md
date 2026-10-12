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
leetui refresh two-sum
leetui reset two-sum
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

Print the effective configuration and its location with:

```sh
leetui config
```

Update one setting at a time with the convenience commands:

```sh
leetui config language rust
leetui config language c++       # normalized and saved as "cpp"
leetui config workspace '~/leetcode'
leetui config editor 'nvim {file}'
leetui config editor 'code --wait {dir}'
```

### `config.toml` format

The file contains one TOML table with exactly these three keys:

```toml
default_language = "cpp"
editor = ["nvim", "{file}"]
workspace = "/home/alice/leetcode"
```

Unknown keys are rejected so that spelling mistakes do not silently do nothing.

| Key | Type | Meaning |
|---|---|---|
| `default_language` | string | LeetCode language slug used when creating a new workspace |
| `editor` | array of strings | One editor command represented as an argument vector |
| `workspace` | path string | Parent directory for generated problem workspaces |

The CLI key `language` updates the TOML key `default_language`. The CLI expands a leading `~` in
the workspace path; a leading `~` in `config.toml` is also expanded when the file is loaded.

On Windows, a TOML literal string avoids escaping backslashes:

```toml
workspace = 'C:\Users\alice\leetcode'
editor = ["code", "--wait", "{dir}"]
```

### Editor command

`editor` is **one command**, not a list of editors. Its first array element is the executable and
the remaining elements are arguments. Therefore this output is correct:

```toml
editor = ["nvim", "{file}"]
```

It means “run `nvim` with the solution path as its argument.” The command
`leetui config editor 'nvim {file}'` parses the quoted value and replaces the entire array. Quote
the whole value so the shell passes it as one CLI argument. For paths or arguments with complex
quoting, edit the TOML array directly instead.

Supported placeholders are:

- `{file}` — absolute path to the solution file.
- `{dir}` — absolute path to the problem workspace.

Placeholders are replaced in individual arguments. No shell is invoked, so pipes, redirects,
command substitution, and shell variable expansion are not supported. If no placeholder appears,
the solution path is appended automatically. An empty array (`editor = []`) uses `$VISUAL`, then
`$EDITOR`; those environment values are tokenized using the same command-line parser.

### Language values

Languages are stored using the `langSlug` returned by LeetCode. Common values include:

| Language | Value |
|---|---|
| C++ | `cpp` |
| C | `c` |
| C# | `csharp` |
| Java | `java` |
| Python 3 | `python3` |
| Python | `python` |
| JavaScript | `javascript` |
| TypeScript | `typescript` |
| Go | `golang` |
| Rust | `rust` |
| Kotlin | `kotlin` |
| Swift | `swift` |
| Ruby | `ruby` |
| PHP | `php` |
| MySQL / MS SQL / Oracle | `mysql` / `mssql` / `oraclesql` |

Common aliases such as `c++`, `c#`, `go`, `js`, `ts`, `py`, and `rs` are accepted by configuration
commands and `--lang`, then normalized to LeetCode's value. Not every problem supplies starter code
for every language; an error now lists the languages available for that problem.

The configured default applies only when creating a workspace. An existing workspace keeps the
language recorded in its `.leetui.toml`. Switch one deliberately with, for example:

```sh
leetui edit two-sum --lang c++
```

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
are preserved. Solution files are also created only once. If an explicit language switch would
reuse the same extension (for example, `python3` to `python`), leetui creates a qualified file such
as `solution.python.py` rather than treating the existing `solution.py` as code for the new
language.

Refresh a prepared statement from LeetCode with `leetui refresh TARGET`, or press `R` while viewing
the problem in the TUI. If `problem.md` already matches the generated statement, no file is written.
If it differs, leetui asks for confirmation, saves the previous contents under
`.leetui-backups/problem.md.bak` (using a numbered suffix when needed), and atomically replaces the
statement. Use `--yes` to skip the CLI confirmation. Missing statement files are recreated without
a backup.

Restore the active solution to LeetCode's starter code with `leetui reset TARGET`, or press `X`
while viewing the problem in the TUI. Modified source is saved under
`.leetui-backups/<solution-file>.bak` before the solution is atomically reset. Numbered suffixes
preserve earlier backups. An unchanged solution is left alone, a missing active solution is
recreated, and other language solution files are never modified. Use `--yes` to skip the CLI
confirmation.

## TUI keys

| Key | Action |
|---|---|
| `/` | Search |
| `j` / `k` | Move or scroll |
| `Enter` | Open problem |
| `Tab` | Load official editorial |
| `i` | Preview the first statement image |
| `p` | Prepare workspace files without opening an editor |
| `R` | Refresh `problem.md`, backing up a differing file |
| `X` | Reset the active solution to starter code with a backup |
| `e` | Open solution in configured editor |
| `r` | Run the sample testcase |
| `S` | Confirm and submit the current solution |
| `m` | Show personal submissions |
| `Esc` | Close image or go back |
| `q` | Back or quit |

Use the CLI `run --input` or `run --input-file` commands for custom multiline testcases.

Pressing `p` creates or reopens the problem workspace (`problem.md`, the starter solution, and
`.leetui.toml`) and leaves the TUI running. This is useful when the editor is already open in
another terminal or tmux pane. Existing solution and notes files are preserved.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo audit
```

Authenticated judge calls are deliberately not part of the automated test suite.
