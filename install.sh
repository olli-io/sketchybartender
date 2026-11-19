#!/bin/bash

set -e

# Check if running on macOS
if [[ "$(uname)" != "Darwin" ]]; then
    echo "Error: This script is only for macOS"
    exit 1
fi

# Check if Rust/Cargo is available
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo not found. Please install Rust first."
    exit 1
fi

# Determine installation directory
INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/sketchybar"
BARTENDER_BINARY="sketchybartender"
CLI_BINARY="sketchycli"

# Create installation directory if it doesn't exist
mkdir -p "${INSTALL_DIR}"
mkdir -p "${CONFIG_DIR}"

# Get the directory where the script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Lucide icon font (https://lucide.dev), used for all bar icons. Codepoints
# change between Lucide releases, so the version is pinned to the one the
# daemon's glyphs come from.
LUCIDE_VERSION="1.74.0"
FONT_DIR="${HOME}/Library/Fonts"
if [[ ! -f "${FONT_DIR}/lucide.ttf" ]]; then
    echo "==> Installing Lucide icon font ${LUCIDE_VERSION}..."
    FONT_TMP="$(mktemp -d)"
    curl -fsSL -o "${FONT_TMP}/lucide-font.zip" \
        "https://github.com/lucide-icons/lucide/releases/download/${LUCIDE_VERSION}/lucide-font-${LUCIDE_VERSION}.zip"
    unzip -qo "${FONT_TMP}/lucide-font.zip" -d "${FONT_TMP}"
    mkdir -p "${FONT_DIR}"
    cp -f "${FONT_TMP}/lucide-font/lucide.ttf" "${FONT_DIR}/lucide.ttf"
    rm -rf "${FONT_TMP}"
    echo "✓ Lucide font installed to ${FONT_DIR}/lucide.ttf"
fi

echo "==> Building sketchybartender..."

# Check if sketchybartender Cargo.toml exists
if [[ ! -f "${SCRIPT_DIR}/sketchybartender/Cargo.toml" ]]; then
    echo "Error: Cargo.toml not found in ${SCRIPT_DIR}/sketchybartender"
    exit 1
fi

# Build the Rust project in release mode
cd "${SCRIPT_DIR}/sketchybartender"
if ! cargo build --release; then
    echo "Error: Bartender compilation failed"
    exit 1
fi

# Copy the binaries
cp -f "${SCRIPT_DIR}/sketchybartender/target/release/${BARTENDER_BINARY}" "${INSTALL_DIR}/${BARTENDER_BINARY}"
cp -f "${SCRIPT_DIR}/sketchybartender/target/release/${CLI_BINARY}" "${INSTALL_DIR}/${CLI_BINARY}"
chmod +x "${INSTALL_DIR}/${BARTENDER_BINARY}"
chmod +x "${INSTALL_DIR}/${CLI_BINARY}"

# Copy the defaultconfiguration file
cp -f "${SCRIPT_DIR}/sketchybartender.default.json" "${CONFIG_DIR}/sketchybartender.default.json"

echo "✓ sketchybartender installed to ${INSTALL_DIR}/${BARTENDER_BINARY}"
echo "✓ sketchycli installed to ${INSTALL_DIR}/${CLI_BINARY}"

cd "${SCRIPT_DIR}"

# Copy sketchybarrc to config directory
    if [[ -f "${CONFIG_DIR}/sketchybarrc" ]]; then
        cp -f "${CONFIG_DIR}/sketchybarrc" "${CONFIG_DIR}/sketchybarrc.bak"
    fi
    cp -f "${SCRIPT_DIR}/sketchybarrc" "${CONFIG_DIR}/sketchybarrc"

if command -v sketchybar &> /dev/null; then
    echo "✓ Reloading sketchybar..."
    sketchybar --reload || echo "Warning: sketchybar --reload failed (is sketchybar running?)"
else
    echo "Warning: sketchybar not found on PATH, skipping reload"
fi

# Install the LaunchAgent that starts sketchybartender at login, and (re)start
# the daemon through it so the new binary runs now.
AGENT_LABEL="local.sketchybartender"
AGENT_DIR="${HOME}/Library/LaunchAgents"
AGENT_PLIST="${AGENT_DIR}/${AGENT_LABEL}.plist"
AGENT_TARGET="gui/$(id -u)/${AGENT_LABEL}"
mkdir -p "${AGENT_DIR}"
launchctl bootout "${AGENT_TARGET}" 2>/dev/null || true
for _ in 1 2 3 4 5 6 7 8 9 10; do
    launchctl print "${AGENT_TARGET}" &> /dev/null || break
    sleep 0.3
done
# A daemon started by an older sketchybarrc is not under launchd.
pkill -f "${INSTALL_DIR}/${BARTENDER_BINARY}" 2>/dev/null || true
cp -f "${SCRIPT_DIR}/${AGENT_LABEL}.plist" "${AGENT_PLIST}"
if launchctl bootstrap "gui/$(id -u)" "${AGENT_PLIST}"; then
    echo "✓ sketchybartender LaunchAgent installed and started (${AGENT_PLIST})"
else
    echo "Warning: could not load ${AGENT_PLIST}; load it with: launchctl bootstrap gui/\$(id -u) ${AGENT_PLIST}"
fi

echo ""
echo "==> Installation complete!"
echo ""
echo "Installed binaries:"
echo "  - ${INSTALL_DIR}/${BARTENDER_BINARY}"
echo "  - ${INSTALL_DIR}/${CLI_BINARY}"
echo ""
echo "Configuration:"
echo "  - ${CONFIG_DIR}/sketchybartenderrc.(default).json"
echo "  - ${CONFIG_DIR}/sketchybarrc"
echo ""
echo "Login entry:"
echo "  - ${AGENT_PLIST}"
echo ""