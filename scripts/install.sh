#!/bin/bash
# =============================================================================
#  xezim — Production-grade installer for Linux & macOS
#
#  Usage:
#    curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh
#    XEZIM_TAG=v0.9.6 curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh
#
#  What this does:
#    1. Checks prerequisites (git, curl/wget)
#    2. Checks macOS Xcode CLI tools (macOS only)
#    3. Installs Rust via rustup (if missing)
#    4. Clones xezim-core + xezim side-by-side into ~/xezim-workspace
#    5. Detects the latest release tag from git (if XEZIM_TAG not set)
#    6. Builds xezim in release mode
#    7. Installs xezim globally (symlink + PATH)
#    8. Runs a quick smoke test
# =============================================================================

set -euo pipefail

# ---- Parse --local flag ----
LOCAL_MODE=false
for arg in "$@"; do
    case "$arg" in
        --local) LOCAL_MODE=true ;;
    esac
done

# ---- Config ----
WORKSPACE="$HOME/xezim-workspace"
GITHUB_ORG="aionhw"
REPO_CORE="xezim-core"
REPO_MAIN="xezim"
BIN_DIR_REL="target/release"

OS=""
ARCH=""

# ---- Colors ----
BOLD='\033[1m'
NC='\033[0m'

log()  { echo -e " ✅ $1"; }
warn() { echo -e " ⚠️  $1"; }
info() { echo -e " ➡️  $1"; }
fail() { echo -e " ❌ $1"; exit 1; }

# ---- Cleanup handler ----
cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo ""
        warn "Installation did not complete successfully (exit code: $exit_code)."
        echo "   Check the output above for details."
    fi
}
trap cleanup EXIT

# ---- Download helper (curl → wget fallback) ----
download() {
    local url="$1"
    local output="${2:--}"
    if command -v curl &>/dev/null; then
        if [ "$output" = "-" ]; then curl -fsSL "$url"; else curl -fsSL -o "$output" "$url"; fi
    elif command -v wget &>/dev/null; then
        if [ "$output" = "-" ]; then wget -qO- "$url"; else wget -qO "$output" "$url"; fi
    else
        fail "Cannot download: neither curl nor wget is available."
    fi
}

# ---- Pre-flight checks ----
preflight() {
    info "Checking prerequisites..."

    if [ -z "${HOME:-}" ]; then
        fail "HOME environment variable is not set."
    fi

    if ! command -v git &>/dev/null; then
        fail "git is required. Install it and re-run this script."
    fi

    if ! command -v curl &>/dev/null && ! command -v wget &>/dev/null; then
        fail "curl or wget is required. Install one and re-run this script."
    fi

    # macOS: Check Xcode CLI tools
    if [ "$(uname -s)" = "Darwin" ]; then
        if ! xcode-select -p &>/dev/null; then
            warn "Xcode CLI tools not found."
            info "Install with: xcode-select --install"
            info "After installation completes, re-run this script."
            exit 1
        fi
    fi

    log "All prerequisites met."
}

# ---- Detect OS ----
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        CYGWIN*|MINGW*|MSYS*) fail "Windows detected. Please use the PowerShell installer: irm https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.ps1 | iex" ;;
        *)       fail "Unsupported OS: $(uname -s). This script supports Linux and macOS only." ;;
    esac
}

# ---- Install Rust toolchain ----
install_rust() {
    info "Checking Rust toolchain..."
    if command -v rustc &>/dev/null; then
        RUST_VER=$(rustc --version 2>/dev/null | awk '{print $2}') || true
        if [ -n "$RUST_VER" ]; then
            log "Rust already installed: v${RUST_VER}"
            RUST_MAJOR=$(echo "$RUST_VER" | cut -d. -f1)
            RUST_MINOR=$(echo "$RUST_VER" | cut -d. -f2)
            if [ "$RUST_MAJOR" -lt 1 ] || ([ "$RUST_MAJOR" -eq 1 ] && [ "$RUST_MINOR" -lt 75 ]); then
                warn "Rust ${RUST_VER} is below minimum 1.75. Updating..."
                rustup update stable
            fi
        else
            # rustup shim exists but no toolchain installed — set default
            rustup default stable 2>/dev/null || true
            RUST_VER=$(rustc --version 2>/dev/null | awk '{print $2}') || true
            [ -n "$RUST_VER" ] && log "Rust ready: v${RUST_VER}"
        fi
    else
        warn "Rust not found. Installing via rustup..."
        download "https://sh.rustup.rs" | sh -s -- -y --quiet || {
            fail "rustup installation failed. Install manually: https://rustup.rs"
        }
        log "Rust installed."
    fi

    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi
    if ! command -v cargo &>/dev/null; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    if ! command -v cargo &>/dev/null; then
        fail "Rust installed but cargo not in PATH. Add ~/.cargo/bin to your PATH and re-run."
    fi
    log "Rust ready: $(rustc --version)"
}

# ---- Clone / update repositories ----
clone_repos() {
    if $LOCAL_MODE; then
        info "Using local checkout..."
        return
    fi

    info "Setting up workspace at ${WORKSPACE}..."
    mkdir -p "$WORKSPACE"

    for repo in "$REPO_CORE" "$REPO_MAIN"; do
        local repo_url="https://github.com/${GITHUB_ORG}/${repo}.git"
        local repo_dir="$WORKSPACE/$repo"
        if [ -d "$repo_dir" ]; then
            info "Updating $repo..."
            cd "$repo_dir"
            git fetch --tags --quiet 2>/dev/null || true
        else
            info "Cloning $repo..."
            git clone --quiet --depth 1 "$repo_url" "$repo_dir"
            cd "$repo_dir" && git fetch --depth 1 --tags --quiet 2>/dev/null || true
        fi
        log "$repo ready."
    done

    if [ ! -d "$WORKSPACE/$REPO_CORE/xezim-parser" ]; then
        fail "xezim-core/xezim-parser not found! Clone may be incomplete."
    fi
    log "Workspace ready."
}

# ---- Verify workspace (local or cloned) ----
verify_workspace() {
    if $LOCAL_MODE; then
        local local_dir
        local_dir="$(cd "$(dirname "$0")/.." && pwd)"
        # Ensure xezim-core is a sibling
        if [ ! -d "$local_dir/../xezim-core/xezim-parser" ]; then
            info "Cloning xezim-core as sibling..."
            git clone --quiet --depth 1 "https://github.com/${GITHUB_ORG}/xezim-core.git" "$local_dir/../xezim-core"
        fi
        WORKSPACE="$(cd "$local_dir/.." && pwd)"
        log "Local workspace: $WORKSPACE"
    fi
}

# ---- Detect tag and checkout ----
resolve_tag() {
    info "Resolving version..."

    XEZIM_TAG_EXPLICIT=false
    if [ -z "${XEZIM_TAG:-}" ]; then
        info "Detecting latest release tag..."
        cd "$WORKSPACE/$REPO_MAIN"
        LATEST_TAG=$(git tag --sort=-creatordate | head -1)
        if [ -n "$LATEST_TAG" ]; then
            XEZIM_TAG="$LATEST_TAG"
        else
            XEZIM_TAG="main"
            info "No tags found, using 'main' branch."
        fi
    else
        XEZIM_TAG_EXPLICIT=true
    fi

    echo -e " ✅ Using ${BOLD}$XEZIM_TAG${NC}"

    for repo in "$REPO_CORE" "$REPO_MAIN"; do
        cd "$WORKSPACE/$repo"
        # Stash local changes that would prevent checkout
        git stash push -m "xezim-installer" 2>/dev/null || true
        if git checkout "$XEZIM_TAG" 2>/dev/null; then
            log "$repo: checked out $XEZIM_TAG"
        else
            if [ "$XEZIM_TAG_EXPLICIT" = true ]; then
                fail "Tag '$XEZIM_TAG' not found in $repo. Verify the tag name."
            fi
            warn "$XEZIM_TAG not found in $repo, staying on default branch."
        fi
    done
}

# ---- Build (skips if binary is current) ----
build_xezim() {
    BINARY="$WORKSPACE/$REPO_MAIN/$BIN_DIR_REL/xezim"
    cd "$WORKSPACE/$REPO_MAIN"

	# Skip build if binary exists and source hasn't changed since last build
    if [ -f "$BINARY" ]; then
        # Cross-platform: find newer sources (Linux stat -c, macOS stat -f)
        local newest_src=0
        case "$(uname -s)" in
            Darwin) newest_src=$(find src -name '*.rs' -exec stat -f %m {} + 2>/dev/null | sort -rn | head -1 || echo 0) ;;
            *)      newest_src=$(find src -name '*.rs' -exec stat -c %Y {} + 2>/dev/null | sort -rn | head -1 || echo 0) ;;
        esac
        local bin_time
        case "$(uname -s)" in
            Darwin) bin_time=$(stat -f %m "$BINARY" 2>/dev/null || echo 0) ;;
            *)      bin_time=$(stat -c %Y "$BINARY" 2>/dev/null || echo 0) ;;
        esac
        [ "$bin_time" -gt "$newest_src" ] 2>/dev/null && { log "Binary is current (tag: $XEZIM_TAG). Skipping build."; return; }
    fi

    info "Building xezim (release mode)..."
    info "This may take 3-10 minutes."
    echo ""

    # FIXME: remove -Awarnings once source warnings are cleaned up
    RUSTFLAGS="-Awarnings" cargo build --release

    [ -f "$BINARY" ] || fail "Build failed — binary not found at $BINARY."
    log "Build successful! ($(du -h "$BINARY" 2>/dev/null | cut -f1))"
}

# ---- Install system-wide (no sudo required) ----
install_binary() {
    info "Installing xezim globally..."

    BINARY="$WORKSPACE/$REPO_MAIN/$BIN_DIR_REL/xezim"
    INSTALLED=0

    # Strategy 1: Symlink to /usr/local/bin (needs sudo, only tries if passwordless)
    if [ "$INSTALLED" -eq 0 ] && [ -d "/usr/local/bin" ]; then
        if command -v sudo &>/dev/null && sudo -n true 2>/dev/null; then
            sudo ln -sf "$BINARY" /usr/local/bin/xezim 2>/dev/null && INSTALLED=1
        elif [ "$(id -u)" = "0" ]; then
            ln -sf "$BINARY" /usr/local/bin/xezim 2>/dev/null && INSTALLED=1
        fi
    fi

    # Strategy 2: Symlink to ~/.local/bin (no sudo, works for everyone)
    if [ "$INSTALLED" -eq 0 ]; then
        LOCAL_BIN="${XDG_DATA_HOME:-$HOME/.local}/bin"
        mkdir -p "$LOCAL_BIN"
        ln -sf "$BINARY" "$LOCAL_BIN/xezim" 2>/dev/null || true
        export PATH="$LOCAL_BIN:$PATH"
        _add_to_shell_config 'export PATH="$PATH:$LOCAL_BIN"'
        command -v xezim &>/dev/null && INSTALLED=1
    fi

    # Strategy 3: Add binary dir to PATH directly (works everywhere)
    if [ "$INSTALLED" -eq 0 ]; then
        XEZIM_DIR="$WORKSPACE/$REPO_MAIN/$BIN_DIR_REL"
        export PATH="$XEZIM_DIR:$PATH"
        _add_to_shell_config "export PATH=\"\$PATH:$XEZIM_DIR\""
        command -v xezim &>/dev/null && INSTALLED=1
    fi

    if [ "$INSTALLED" -eq 1 ]; then
        log "xezim is now available in your terminal."
    else
        warn "Using xezim from workspace directory."
        export PATH="$WORKSPACE/$REPO_MAIN/$BIN_DIR_REL:$PATH"
    fi
}

# ---- Helper: add line to shell config ----
_add_to_shell_config() {
    local line="$1"
    local config_file=""

    if [ -n "${SHELL:-}" ]; then
        case "$SHELL" in
            *zsh)  config_file="$HOME/.zshrc" ;;
            *bash) config_file="$HOME/.bashrc" ;;
            *fish) config_file="$HOME/.config/fish/config.fish" ;;
        esac
    fi

    if [ -z "$config_file" ] || [ ! -f "$config_file" ]; then
        for f in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.profile" "$HOME/.config/fish/config.fish"; do
            [ -f "$f" ] && { config_file="$f"; break; }
        done
    fi

    [ -z "$config_file" ] && config_file="$HOME/.profile"

    if ! grep -q "xezim" "$config_file" 2>/dev/null; then
        echo "" >> "$config_file"
        echo "# xezim" >> "$config_file"
        echo "$line" >> "$config_file"
        log "Added xezim to $config_file"
    fi
}

# ---- Smoke test ----
smoke_test() {
    info "Running smoke test..."
    BINARY="$WORKSPACE/$REPO_MAIN/$BIN_DIR_REL/xezim"

    if "$BINARY" --help &>/dev/null; then
        log "xezim --help works!"
    else
        warn "xezim --help returned non-zero (may still be OK)."
    fi

    local version_output
    version_output=$("$BINARY" --version 2>/dev/null || true)
    [ -n "$version_output" ] && log "$version_output"
}

# =============================================================================
#  Main
# =============================================================================

preflight
OS=$(detect_os)
ARCH=$(uname -m)

echo ""
echo -e "${BOLD}🚀  xezim Installer — $OS ($ARCH)${NC}"
echo "   Workspace: $WORKSPACE"
echo ""

install_rust
clone_repos
verify_workspace
resolve_tag

# Quick exit if xezim already matches the target tag
if command -v xezim &>/dev/null; then
    local_ver=$(xezim --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
    tag_ver=$(echo "$XEZIM_TAG" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
    if [ -n "$local_ver" ] && [ "$local_ver" = "$tag_ver" ]; then
        echo ""
        echo -e "${BOLD}🎉  xezim v$local_ver is already installed and up to date!${NC}"
        echo ""
        echo "   Workspace: $WORKSPACE"
        echo ""
        echo "   Update later to check for new versions:"
        echo "      curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh"
        echo ""
        exit 0
    fi
fi

build_xezim
install_binary
smoke_test

echo ""
echo -e "${BOLD}🎉  Installation Complete!${NC}"
echo ""
echo -e "   ${BOLD}Version:${NC}    $XEZIM_TAG"
echo -e "   ${BOLD}Binary:${NC}     $BINARY"
echo -e "   ${BOLD}Workspace:${NC}  $WORKSPACE"
echo ""
echo "   Open a new terminal and run:"
echo "      xezim --help"
echo ""
echo "   Example:"
echo "      cd $WORKSPACE/$REPO_MAIN && xezim examples/full_adder.sv examples/tb_adder.sv"
echo ""
echo "   Update later:"
echo "      curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh"
echo ""