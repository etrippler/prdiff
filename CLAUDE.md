# prdiff

Terminal PR diff viewer - shows branch changes like GitHub's "Files changed" tab.

## Build & Run

```bash
cargo build --release
./target/release/prdiff              # auto-detect develop/main/master
./target/release/prdiff main         # positional arg
./target/release/prdiff -b main      # flag form
./target/release/prdiff --help       # show usage
```

## Architecture

Multi-file Rust TUI using:
- `ratatui` - terminal UI framework
- `crossterm` - terminal input/events
- `syntect` - syntax highlighting
- `anyhow` - error handling

## Key Components

- `App` - main state: files, tree, cursor, caches
- `TreeNode` - file tree with directory collapsing
- `Highlighter` - syntect wrapper for diff highlighting
- `GitWatcher` - background thread for git polling
- `run_app()` - main loop: handle events → check updates → draw

## Event Loop Design

The UI follows an event-first architecture for responsiveness:

1. **Drain all pending events** - process input immediately, never block
2. **Check for git updates** - receive from background watcher (non-blocking)
3. **Render if needed** - only redraw when state changes

Key optimizations:
- Visible items cached with version tracking (only rebuild when tree/expand changes)
- `needs_redraw` flag prevents unnecessary renders
- Short poll timeout (50ms) balances responsiveness with CPU usage

## Background Git Watcher

Git operations run in a separate thread (`watcher.rs`) to never block the UI:

- Polls every 200ms for changes (HEAD, index, file mtimes, git status)
- Sends updates via `mpsc` channel
- Main thread receives non-blocking with `try_recv()`

## Terminal Handling

Proper terminal setup/teardown in `TerminalGuard`:

1. Setup: raw mode → alternate screen → mouse capture → kitty keyboard enhancement
2. Teardown: pop keyboard enhancement → disable mouse → **drain pending input** → leave screen → disable raw mode

The drain step is critical - it consumes any mouse events still in the buffer to prevent escape sequence garbage on exit.

## Keyboard Protocol

Uses the kitty keyboard protocol (`DISAMBIGUATE_ESCAPE_CODES`) for unambiguous key handling.

Supported terminals: ghostty, kitty, WezTerm, Alacritty, iTerm2, foot, rio.

**Note:** Esc is NOT used as an exit key because some terminals/escape sequences can be misinterpreted as Esc, causing unexpected exits. Use `q` or `Ctrl+C` instead.

## Controls

- `j/k` or arrows: navigate files
- `h/l`: collapse/expand directories
- `Space`/`Enter`: toggle expand/collapse directories
- `Enter` on file: open in editor
- `J/K`: scroll diff
- `<`/`>`: resize panel split (shrink/grow file tree)
- Mouse: click files, scroll diff panel
- `q`/`Ctrl+C`: quit

## Editor

Opens files with `PRDIFF_EDITOR`, falling back to `EDITOR`, then `zed` as default.

## Debugging

Set `PRDIFF_LOG=/path/to/file` to enable logging (appends):
```bash
PRDIFF_LOG=/tmp/prdiff.log ./target/release/prdiff
```

## Rust Patterns

### Avoid `.unwrap()` in control flow
Prefer pattern matching or `?` over `.unwrap()`. Instead of:
```rust
let path = parts.last().unwrap().to_string();
```
Use:
```rust
let Some(path) = parts.last() else { continue };
```
Reserve `.unwrap()` for cases where failure is truly impossible and obvious from context.

### Clone discipline
Don't reach for `.clone()` just to satisfy the borrow checker. Before cloning, consider:
- Can you restructure to use borrows instead?
- Can functions take `&T` instead of owned `T`?
- Is the data actually needed in multiple places, or can ownership transfer?

When cloning is necessary (e.g., sending data to another thread while keeping a local copy), make the intent clear in the code structure.

### Thread communication
Prefer `mpsc` channels over `Arc<Mutex<T>>` for thread communication. Channels provide cleaner ownership semantics and avoid lock contention. This codebase uses `mpsc` for the git watcher → UI thread communication.
