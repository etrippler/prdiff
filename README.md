# prdiff

A simple terminal UI for viewing all changes on your branch compared to the base branch.

## Why?

When developing with AI agents, the traditional commit-by-commit view isn't always helpful. You often just want to see "what are all the actual changes on this branch?" — regardless of whether they're committed, staged, or untracked.

prdiff shows your branch diff the way GitHub's "Files changed" tab does: everything that's different from the base branch, all in one view.

## Install

```bash
cargo build --release
# optionally symlink to PATH
ln -s $(pwd)/target/release/prdiff ~/.local/bin/prdiff
```

## Usage

```bash
prdiff              # auto-detects develop/main/master as base
prdiff main         # explicit base branch
prdiff -b feature   # flag form
prdiff -t light     # use light theme
```

## Configuration

| Variable | Description |
|----------|-------------|
| `PRDIFF_THEME` | Color theme: `dark` (default) or `light` |
| `PRDIFF_EDITOR` | Editor for opening files (falls back to `EDITOR`, then `zed`) |

## Controls

- `j/k` or arrows: navigate files
- `h/l`: collapse/expand directories
- `J/K`: scroll diff
- `Enter`: open file in editor
- Mouse: click files, scroll diff
- `q` or `Ctrl+C`: quit

## Status

100% vibecoded. Works, but a bit buggy. Good enough for daily use.
