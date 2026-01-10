# sketchybar-employees

Helper daemon and cli script to run my sketchybar setup (WIP).

1. Sketchybartender (rust) handles updating all information in sketchybar.
2. Sketchycli (rust) passes on cli commands to sketchybartender, when triggered by aerospace (window manager) or sketchybar.

## Installation

### Deps:

```bash
brew install rust
```

```bash
brew install FelixKratz/formulae/sketchybar
```

```bash
brew install --cask nikitabobko/tap/aerospace
```

### 2. Install sketchybar employees

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/olli-io/sketchybar-employees/HEAD/install.sh)"
```

This will:

- Build and install `sketchybartender` (Rust daemon)
- Build and install `sketchycli` (CLI tool)
- Copy configuration files to `~/.config/sketchybar/`
- Backup your existing sketchybarrc if present

At this point the sketchybar may be a bit empty. Add the following to aerospace.toml and run `aerospace reload-config`

```
exec-on-workspace-change = [
    '/bin/bash', '-c',
    'sketchycli on-workspace-changed'
]

on-focus-changed = [
    'exec-and-forget sketchycli on-focus-changed'
]
```

### 3. Ensure PATH is Set

Make sure `~/.local/bin` is in your PATH. Add this to your shell config if needed:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## What Gets Installed

Binaries:

- `~/.local/bin/sketchybartender`
- `~/.local/bin/sketchycli`

Configuration:

- `~/.config/sketchybar/sketchybarrc`
- `~/.config/sketchybar/sketchybartenderrc`

## Usage

The daemons are automatically started by sketchybar. You can configure update intervals in `~/.config/sketchybar/sketchybartenderrc`.

## Uninstall

To fire sketchybar-employees:

```bash

# Remove the binaries
rm -f ~/.local/bin/sketchybartender
rm -f ~/.local/bin/sketchycli

# Remove configuration files (optional - keeps your config)
rm -f ~/.config/sketchybar/sketchybartenderrc
# rm -f ~/.config/sketchybar/sketchybarrc  # Uncomment if you want to remove sketchybarrc too
```
