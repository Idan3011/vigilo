#!/usr/bin/env bash
set -e

REPO="https://github.com/Idan3011/vigilo"
DOWNLOAD_BASE="$REPO/releases/latest/download"

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

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)        ARCH_TAG="x86_64" ;;
  aarch64|arm64) ARCH_TAG="aarch64" ;;
  *)
    echo "Unsupported architecture: $ARCH"
    echo "Install Rust and build from source: cargo install --git $REPO"
    exit 1
    ;;
esac


ARTIFACT="vigilo-${ARCH_TAG}-${PLATFORM}"
DOWNLOAD_URL="${DOWNLOAD_BASE}/${ARTIFACT}"

if [ -d "$HOME/.cargo/bin" ]; then
  INSTALL_DIR="$HOME/.cargo/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

BINARY="$INSTALL_DIR/vigilo"

echo "  Downloading from: $DOWNLOAD_URL"

download_ok=false
if command -v curl &>/dev/null; then
  if curl -fsSL "$DOWNLOAD_URL" -o "$BINARY" 2>/dev/null; then
    download_ok=true
  fi
elif command -v wget &>/dev/null; then
  if wget -q "$DOWNLOAD_URL" -O "$BINARY" 2>/dev/null; then
    download_ok=true
  fi
fi

if [ "$download_ok" = true ]; then
  if ! file "$BINARY" | grep -qE 'ELF|Mach-O'; then
    echo "  Downloaded file is not a valid binary."
    echo "  The release may not exist yet for $ARTIFACT."
    rm -f "$BINARY"
    exit 1
  fi
  chmod +x "$BINARY"
  echo "  ✓ downloaded pre-built binary"
else
  echo "  Pre-built binary not available for $ARTIFACT"
  if command -v cargo &>/dev/null; then
    echo "  Building from source via cargo (this takes ~30s)..."
    cargo install --git "$REPO" --quiet
    BINARY="$(which vigilo 2>/dev/null || echo "$HOME/.cargo/bin/vigilo")"
  else
    echo "  Neither pre-built binary nor cargo available."
    echo "  Install Rust first: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
  fi
fi

if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
  echo ""
  echo "  Add to your shell profile (~/.bashrc or ~/.zshrc):"
  echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
  echo ""
fi

echo "  ✓ vigilo installed: $BINARY"

# ── Shell completions ────────────────────────────────────────────────────────
if [ -t 0 ]; then
  SHELL_NAME="$(basename "$SHELL" 2>/dev/null || echo "")"
  case "$SHELL_NAME" in
    zsh)  RC_FILE="$HOME/.zshrc"; COMP_LINE='eval "$(vigilo completions zsh)"' ;;
    bash) RC_FILE="$HOME/.bashrc"; COMP_LINE='eval "$(vigilo completions bash)"' ;;
    *)
      if [ -f "$HOME/.bashrc" ]; then
        RC_FILE="$HOME/.bashrc"; COMP_LINE='eval "$(vigilo completions bash)"'
      elif [ -f "$HOME/.zshrc" ]; then
        RC_FILE="$HOME/.zshrc"; COMP_LINE='eval "$(vigilo completions zsh)"'
      fi
      ;;
  esac

  if [ -n "$RC_FILE" ]; then
    if grep -qF "vigilo completions" "$RC_FILE" 2>/dev/null; then
      echo "  ✓ shell completions already configured"
    else
      printf "  Enable tab completions in %s? [Y/n] " "$(basename "$RC_FILE")"
      read -r answer
      case "$answer" in
        n|N|no|No) echo "  skipped — run 'vigilo completions $SHELL_NAME' manually anytime" ;;
        *)
          echo "" >> "$RC_FILE"
          echo "# vigilo shell completions" >> "$RC_FILE"
          echo "$COMP_LINE" >> "$RC_FILE"
          echo "  ✓ shell completions added to $(basename "$RC_FILE")"
          ;;
      esac
    fi
  fi
fi

echo ""

if [ -t 0 ]; then
  "$BINARY" setup
else
  echo "  Run 'vigilo setup' to complete configuration."
fi
