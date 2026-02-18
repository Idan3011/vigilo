#!/usr/bin/env bash
set -e

REPO="https://github.com/your-org/vigilo"   # update before publishing

echo ""
echo "vigilo installer"
echo ""

# ── Detect OS ────────────────────────────────────────────────────────────────
OS="$(uname -s)"
case "$OS" in
  Linux*)  PLATFORM="linux" ;;
  Darwin*) PLATFORM="macos" ;;
  *)
    echo "Unsupported OS: $OS"
    echo "Please install manually: cargo install --git $REPO"
    exit 1
    ;;
esac

# ── Install binary ───────────────────────────────────────────────────────────
if command -v cargo &>/dev/null; then
  echo "  Installing via cargo (this takes ~30s)..."
  cargo install --git "$REPO" --quiet
  BINARY="$(which vigilo 2>/dev/null || echo "$HOME/.cargo/bin/vigilo")"
else
  echo "  Cargo not found. Attempting to download pre-built binary..."

  ARCH="$(uname -m)"
  case "$ARCH" in
    x86_64)  ARCH_TAG="x86_64" ;;
    aarch64|arm64) ARCH_TAG="aarch64" ;;
    *)
      echo "  Unsupported architecture: $ARCH"
      echo "  Install Rust and re-run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
      exit 1
      ;;
  esac

  BINARY="$HOME/.local/bin/vigilo"
  mkdir -p "$HOME/.local/bin"

  DOWNLOAD_URL="$REPO/releases/latest/download/vigilo-${ARCH_TAG}-${PLATFORM}"
  echo "  Downloading from: $DOWNLOAD_URL"

  if command -v curl &>/dev/null; then
    curl -fsSL "$DOWNLOAD_URL" -o "$BINARY"
  elif command -v wget &>/dev/null; then
    wget -q "$DOWNLOAD_URL" -O "$BINARY"
  else
    echo "  Neither curl nor wget found. Install Rust:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
  fi

  chmod +x "$BINARY"

  # Add ~/.local/bin to PATH if needed
  if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
    echo ""
    echo "  Add to your shell profile (~/.bashrc or ~/.zshrc):"
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
  fi
fi

echo "  ✓ vigilo installed: $(vigilo --version 2>/dev/null || echo "$BINARY")"
echo ""

# ── Run interactive setup ────────────────────────────────────────────────────
vigilo setup
