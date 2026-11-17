# sketchybar-employees

Helper daemon and cli script to run my sketchybar setup (WIP).

1. Sketchybartender (rust) handles updating all information in sketchybar.
2. Sketchycli (rust) passes on cli commands to sketchybartender, when triggered by sketchybar.

Supported window managers: [AeroSpace](https://github.com/nikitabobko/AeroSpace) (default) and [Rift](https://github.com/acsandmann/rift).

## Installation

### 1. Deps

```bash
brew install rust
```

```bash
brew install FelixKratz/formulae/sketchybar
```

Install one window manager:

```bash
brew install --cask nikitabobko/tap/aerospace
```

```bash
brew install acsandmann/tap/rift
```

### 2. Install sketchybartender

```bash
git clone https://github.com/olli-io/sketchybar-employees
cd sketchybar-employees
./install.sh
```

This will:

- Install the [Lucide](https://lucide.dev) icon font (pinned release) to `~/Library/Fonts` if it is missing. The bar icons are Lucide glyphs.
- Build and install `sketchybartender` (Rust daemon)
- Build and install `sketchycli` (CLI tool)
- Copy configuration files to `~/.config/sketchybar/`
- Backup your existing sketchybarrc if present

### 3. Select the window manager

Set `window_manager` in `~/.config/sketchybar/sketchybartender.json` and restart sketchybar (`sketchybar --reload`):

```json
{
  "window_manager": "rift"
}
```

| Value | Connection |
| --- | --- |
| `"aerospace"` (default) | AeroSpace Unix socket (`/tmp/bobko.aerospace-$USER.sock`) |
| `"rift"` | Rift Mach IPC service (`git.acsandmann.rift`, or `$RIFT_BS_NAME`) |

The daemon subscribes to the window manager's events directly. You do not need callbacks in `aerospace.toml` or `rift-cli subscribe` entries.

Rift notes:

- The bar shows a Rift workspace by its name if the name is one character, `1`-`9` or `a`-`z` (set `workspace_names` in the Rift config). Otherwise it shows the 1-based workspace index. A click on an item switches to that workspace.
- In Rift's `per_display` mode, each display has a workspace "1". Sketchybar can show an item on one display only, so the bar shows the focused display's workspace. Use `global` mode to see all workspaces on a multi-display setup.

### 4. Ensure PATH is Set

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
