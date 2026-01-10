#!/bin/bash

set -e

# Check if running on macOS
if [[ "$(uname)" != "Darwin" ]]; then
    echo "Error: This script is only for macOS"
    exit 1
fi

# Check if sketchybar is installed
if ! command -v sketchybar &> /dev/null; then
    echo "Error: sketchybar not found. Please install sketchybar first:"
    echo "  brew install FelixKratz/formulae/sketchybar"
    exit 1
fi

# Check if Rust/Cargo is available
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo not found. Please install Rust:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
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

# Copy the configuration file
if [[ -f "${SCRIPT_DIR}/sketchybartenderrc" ]]; then
    cp -f "${SCRIPT_DIR}/sketchybartenderrc" "${CONFIG_DIR}/sketchybartenderrc"
    echo "✓ Configuration installed to ${CONFIG_DIR}/sketchybartenderrc"
fi

echo "✓ sketchybartender installed to ${INSTALL_DIR}/${BARTENDER_BINARY}"
echo "✓ sketchycli installed to ${INSTALL_DIR}/${CLI_BINARY}"

cd "${SCRIPT_DIR}"

# Copy sketchybarrc to config directory
if [[ -f "${SCRIPT_DIR}/sketchybarrc" ]]; then
    # Backup existing sketchybarrc if it exists
    if [[ -f "${CONFIG_DIR}/sketchybarrc" ]]; then
        cp -f "${CONFIG_DIR}/sketchybarrc" "${CONFIG_DIR}/sketchybarrc.bak"
        echo "✓ Backed up existing sketchybarrc to ${CONFIG_DIR}/sketchybarrc.bak"
    fi
    cp -f "${SCRIPT_DIR}/sketchybarrc" "${CONFIG_DIR}/sketchybarrc"
    echo "✓ sketchybarrc copied to ${CONFIG_DIR}/sketchybarrc"
fi

echo "✓ Reloading sketchybar..."
sketchybar --reload

echo ""
echo "==> Installation complete!"
echo ""
echo "Installed binaries:"
echo "  - ${INSTALL_DIR}/${BARTENDER_BINARY}"
echo "  - ${INSTALL_DIR}/${CLI_BINARY}"
echo ""
echo "Configuration:"
echo "  - ${CONFIG_DIR}/sketchybartenderrc"
echo "  - ${CONFIG_DIR}/sketchybarrc"
echo ""