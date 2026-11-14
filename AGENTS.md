# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 GTK4/libadwaita PDF reader built on MuPDF.

- `src/main.rs` starts the application and handles CLI arguments; `src/check.rs` provides the headless `--check` path.
- `src/document/` contains the MuPDF worker, request/response messages, text model, and outline handling. Keep it independent of `app/` and `view/`.
- `src/app/` contains the application window and actions; `src/view/` contains layout, rendering cache, selection, find, clipboard, and the custom page widget.
- `plans/` records the implementation plan, while `README.md` documents installation and user-facing behavior. `LICENSE` is AGPL-3.0-or-later.

Place new code in the narrowest responsible module and preserve the worker boundary: MuPDF handles stay on the worker thread, and cross-thread messages must remain `Send`.

## Build, Test, and Development Commands

Install the GTK4/libadwaita, fontconfig, clang, and Rust prerequisites listed in `README.md`, then use:

```sh
cargo build --release                         # optimized binary
cargo run -- document.pdf                     # run the GUI with a PDF
cargo run -- --check document.pdf             # headless render/text/search check
cargo test                                    # unit tests
cargo fmt --check                             # formatting verification
cargo clippy --all-targets -- -D warnings     # lint with warnings denied
```

The first build compiles vendored MuPDF sources and may take several minutes.

## Coding Style & Naming Conventions

Use standard `rustfmt` formatting with four-space indentation. Prefer clear snake_case functions/modules, `UpperCamelCase` types, and `SCREAMING_SNAKE_CASE` constants. Keep comments for non-obvious constraints, especially thread ownership, rendering-pixel rounding, and GTK event ordering. Use early returns and small helpers to avoid deep nesting.

## Testing Guidelines

Tests are Rust unit tests colocated with implementation modules under `#[cfg(test)]`; name them after the behavior they protect (for example, `fit_width_uses_common_page_size`). Run `cargo test` for changes to logic, and pair UI/rendering changes with `cargo run -- --check file.pdf` when a suitable PDF is available. Run formatting and Clippy before submitting.

### GUI testing

Visual checks run in a nested headless compositor with screenshots — for example `sway` under `WLR_BACKENDS=headless` and `WLR_LIBINPUT_NO_DEVICES=1`, captured with `grim`. Use whatever is already present; install nothing on the system to do this.

**Never match processes by name when tearing that down.** The nested compositor is the same binary as the developer's own session, and the reader may already be open outside the test, so a name pattern hits the real session or the real app and logs the developer out mid-task. This is not specific to `sway`: it applies to any nested compositor, X server or app instance, and to your own shell, whose command line a `pkill -f` pattern can match too. Record the PID at launch and signal only that PID:

```sh
<compositor> & nested=$!        # …run the checks…
kill "$nested"
```

So: signal PIDs you started, never a name pattern — no `pkill`, `pkill -f`, `pkill -x` or `killall`. If a stray process must be found by name, print the matches for the developer to kill rather than killing them yourself.
