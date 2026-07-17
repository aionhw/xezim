#!/bin/bash
# =============================================================================
#  xezim — Full Install & Build Script for macOS Intel (i7)
#  
#  Usage:
#    chmod +x install_xezim_on_mac.sh
#    ./install_xezim_on_mac.sh
#
#  What this does:
#    1. Installs Xcode CLI tools (if missing)
#    2. Installs Homebrew (if missing)
#    3. Installs system deps (git, pkg-config, libffi)
#    4. Installs Rust via rustup (if missing)
#    5. Clones xezim-core + xezim + UVM 1.2 side-by-side
#    6. Builds xezim in release mode
#    7. Runs a quick smoke test
#    8. Runs UVM simple test (mock)
#    9. Runs UVM complete test (real UVM 1.2 library)
# =============================================================================

set -e  # Exit on any error

# --- Colors for output ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

WORKSPACE="$HOME/xezim-workspace"

log()  { echo -e "${GREEN}[✔]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
info() { echo -e "${CYAN}[→]${NC} $1"; }
fail() { echo -e "${RED}[✘]${NC} $1"; exit 1; }

echo ""
echo "=============================================="
echo "  xezim Installer — macOS Intel (x86_64)"
echo "=============================================="
echo ""

# -------------------------------------------------------
# Step 1: Xcode Command Line Tools
# -------------------------------------------------------
info "Step 1/9: Checking Xcode Command Line Tools..."
if xcode-select -p &>/dev/null; then
    log "Xcode CLI tools already installed."
else
    warn "Xcode CLI tools not found. Installing..."
    xcode-select --install
    echo ""
    warn "A popup should appear. Click 'Install' and wait."
    warn "After installation completes, RE-RUN this script."
    exit 0
fi

# -------------------------------------------------------
# Step 2: Homebrew
# -------------------------------------------------------
info "Step 2/9: Checking Homebrew..."
if command -v brew &>/dev/null; then
    log "Homebrew already installed: $(brew --prefix)"
else
    warn "Homebrew not found. Installing..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    # Intel Macs: brew is at /usr/local/bin/brew
    if [ -f /usr/local/bin/brew ]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi
    log "Homebrew installed."
fi

# -------------------------------------------------------
# Step 3: System dependencies
# -------------------------------------------------------
info "Step 3/9: Installing system dependencies..."

for pkg in git pkg-config libffi; do
    if brew list "$pkg" &>/dev/null; then
        log "$pkg already installed."
    else
        info "Installing $pkg..."
        brew install "$pkg"
        log "$pkg installed."
    fi
done

# Set libffi env vars. The Homebrew prefix differs by architecture
# (/usr/local on Intel, /opt/homebrew on Apple Silicon), so ask brew rather
# than hardcoding it.
LIBFFI_PREFIX="$(brew --prefix libffi 2>/dev/null || echo /usr/local/opt/libffi)"
export LDFLAGS="-L${LIBFFI_PREFIX}/lib"
export CPPFLAGS="-I${LIBFFI_PREFIX}/include"
export PKG_CONFIG_PATH="${LIBFFI_PREFIX}/lib/pkgconfig"

# Persist libffi env to ~/.zshrc if not already there
if ! grep -q 'libffi/lib' ~/.zshrc 2>/dev/null; then
    info "Adding libffi paths to ~/.zshrc..."
    {
        echo ''
        echo '# --- xezim: libffi paths ---'
        echo "export LDFLAGS=\"-L${LIBFFI_PREFIX}/lib\""
        echo "export CPPFLAGS=\"-I${LIBFFI_PREFIX}/include\""
        echo "export PKG_CONFIG_PATH=\"${LIBFFI_PREFIX}/lib/pkgconfig\""
    } >> ~/.zshrc
    log "libffi paths added to ~/.zshrc"
else
    log "libffi paths already in ~/.zshrc"
fi

# -------------------------------------------------------
# Step 4: Rust toolchain
# -------------------------------------------------------
info "Step 4/9: Checking Rust toolchain..."
if command -v rustc &>/dev/null; then
    RUST_VER=$(rustc --version | awk '{print $2}')
    log "Rust already installed: v${RUST_VER}"
    # Check minimum version
    RUST_MAJOR=$(echo "$RUST_VER" | cut -d. -f1)
    RUST_MINOR=$(echo "$RUST_VER" | cut -d. -f2)
    if [ "$RUST_MAJOR" -lt 1 ] || ([ "$RUST_MAJOR" -eq 1 ] && [ "$RUST_MINOR" -lt 75 ]); then
        warn "Rust ${RUST_VER} is below minimum 1.75. Updating..."
        rustup update stable
        log "Rust updated."
    fi
else
    warn "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    log "Rust installed: $(rustc --version)"
fi

# Ensure cargo is in PATH for this session
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi

# -------------------------------------------------------
# Step 5: Clone repositories
# -------------------------------------------------------
info "Step 5/9: Setting up workspace at ${WORKSPACE}..."
mkdir -p "$WORKSPACE"

# Clone xezim-core
if [ -d "$WORKSPACE/xezim-core" ]; then
    log "xezim-core already cloned. Pulling latest..."
    cd "$WORKSPACE/xezim-core" && git pull --ff-only 2>/dev/null || true
else
    info "Cloning xezim-core..."
    git clone https://github.com/aionhw/xezim-core.git "$WORKSPACE/xezim-core"
    log "xezim-core cloned."
fi

# Clone xezim
if [ -d "$WORKSPACE/xezim" ]; then
    log "xezim already cloned. Pulling latest..."
    cd "$WORKSPACE/xezim" && git pull --ff-only 2>/dev/null || true
else
    info "Cloning xezim..."
    git clone https://github.com/aionhw/xezim.git "$WORKSPACE/xezim"
    log "xezim cloned."
fi

# Clone UVM 1.2 (Accellera original — required for UVM complete test)
# IMPORTANT: Do NOT use accellera-official/uvm-core (IEEE 1800.2-2020+),
# xezim's parser doesn't support the complex preprocessor macros in that version.
if [ -d "$WORKSPACE/uvm-1.2" ]; then
    log "UVM 1.2 already cloned."
else
    info "Cloning UVM 1.2 (Accellera original)..."
    git clone https://github.com/gchinna/uvm-1.2.git "$WORKSPACE/uvm-1.2"
    log "UVM 1.2 cloned."
fi

# Verify structure
if [ ! -d "$WORKSPACE/xezim-core/xezim-parser" ]; then
    fail "xezim-core/xezim-parser not found! The clone may be incomplete."
fi
if [ ! -f "$WORKSPACE/uvm-1.2/src/uvm_macros.svh" ]; then
    fail "uvm-1.2/src/uvm_macros.svh not found! UVM clone may be incomplete."
fi
log "Workspace structure verified (xezim-core + xezim + uvm-1.2)."

# -------------------------------------------------------
# Step 6: Build xezim
# -------------------------------------------------------
info "Step 6/9: Building xezim (release mode)..."
info "This may take 3-10 minutes on first build..."
echo ""

cd "$WORKSPACE/xezim"
cargo build --release 2>&1

if [ -f "$WORKSPACE/xezim/target/release/xezim" ]; then
    log "Build successful!"
    log "Binary: $WORKSPACE/xezim/target/release/xezim"
else
    fail "Build failed — binary not found."
fi

# -------------------------------------------------------
# Step 7: Smoke test
# -------------------------------------------------------
info "Step 7/9: Running smoke test..."

"$WORKSPACE/xezim/target/release/xezim" --help &>/dev/null && \
    log "xezim --help works!" || \
    warn "xezim --help returned non-zero (may still be OK)"

# -------------------------------------------------------
# Step 8: UVM simple test (uses bundled mock)
# -------------------------------------------------------
info "Step 8/9: Running UVM simple test (mock)..."

cd "$WORKSPACE/xezim"
cargo test --release --test uvm_integration_tests 2>&1 && \
    log "UVM simple test (mock) PASSED!" || \
    warn "UVM simple test returned non-zero (check output above)"

# -------------------------------------------------------
# Step 9: UVM complete test (real UVM 1.2 library)
# -------------------------------------------------------
info "Step 9/9: Running UVM complete test (real UVM 1.2)..."

cd "$WORKSPACE/xezim"
"$WORKSPACE/xezim/target/release/xezim" \
    -DUVM_NO_DPI \
    +incdir+"$WORKSPACE/uvm-1.2/src" \
    "$WORKSPACE/uvm-1.2/src/uvm_pkg.sv" \
    tests/uvm/uvm_complete_test.sv 2>&1 && \
    log "UVM complete test PASSED!" || \
    warn "UVM complete test returned non-zero (check output above)"

echo ""
echo "=============================================="
echo -e "  ${GREEN}Installation Complete!${NC}"
echo "=============================================="
echo ""
echo "  Workspace:  $WORKSPACE"
echo "  Binary:     $WORKSPACE/xezim/target/release/xezim"
echo "  UVM 1.2:    $WORKSPACE/uvm-1.2/src"
echo ""
echo "  Try running:"
echo "    cd $WORKSPACE/xezim"
echo "    ./target/release/xezim examples/full_adder.sv examples/tb_adder.sv"
echo ""
echo "  Run UVM complete test:"
echo "    cd $WORKSPACE/xezim"
echo "    ./target/release/xezim -DUVM_NO_DPI +incdir+../uvm-1.2/src ../uvm-1.2/src/uvm_pkg.sv tests/uvm/uvm_complete_test.sv"
echo ""
echo "  Run all tests:"
echo "    cd $WORKSPACE/xezim"
echo "    cargo test --release"
echo ""
echo "  (Optional) Install globally:"
echo "    cargo install --path $WORKSPACE/xezim"
echo "    xezim --help"
echo ""
