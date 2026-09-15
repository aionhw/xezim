#!/usr/bin/env bash
# xezim binary release build script.
#
# Runs inside stock manylinux Docker images (Linux) or natively on macOS runners.
# Produces: xezim-<platform>-<version>.tar.gz + .sha256 + manifest.json
# Asset naming follows generic platform conventions so any consumer can match.
#
# Environment (set by the caller / GitHub Actions):
#   VERSION  - the release version (e.g. 0.10.5)
#   ARCH     - normalized architecture (x86_64 | aarch64 | arm64)
#   TARGET   - platform label (manylinux_2_28_x86_64, macos-arm64, etc.)
#   SRC_DIR  - source directory (mounted at /src)
#   WORK_DIR - scratch directory
#   OUT_DIR  - output directory (mounted at /src/dist)

set -euo pipefail

VERSION="${VERSION:?VERSION required}"
ARCH="${ARCH:?ARCH required}"
TARGET="${TARGET:?TARGET required}"
SRC_DIR="${SRC_DIR:-/src}"
WORK_DIR="${WORK_DIR:-/tmp/build}"
OUT_DIR="${OUT_DIR:-/src/dist}"

release_root="${WORK_DIR}/release/xezim"
mkdir -p "${release_root}/bin" "${WORK_DIR}/build" "${OUT_DIR}"

# The workflow provisions Rust when the runner does not ship it; make a
# rustup-installed toolchain discoverable too.
[ -d "${HOME}/.cargo/bin" ] && export PATH="${HOME}/.cargo/bin:${PATH}"

# Use the workspace's target dir explicitly to keep the source pristine.
export CARGO_TARGET_DIR="${WORK_DIR}/cargo-target"

jobs="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"

echo "[build-release] building xezim ${VERSION} for ${TARGET} (arch=${ARCH})"

# xezim does not commit a Cargo.lock, so generate one before the locked
# build. The git-revision pins in Cargo.toml (xezim-core, sv-parser) make
# the generated lockfile deterministic for a given source revision.
( cd "${SRC_DIR}"
  if [ ! -f Cargo.lock ]; then
    echo "[build-release] no Cargo.lock committed, generating one"
    cargo generate-lockfile
  fi
  cargo build --release --locked -j"${jobs}" --bin xezim
)

install -m 0755 "${CARGO_TARGET_DIR}/release/xezim" "${release_root}/bin/xezim"

sha256_file() {
  # sha256sum is coreutils and is not shipped on stock macOS; fall back to
  # shasum there. Both emit "<hex>  <path>".
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${1}"
  else
    shasum -a 256 "${1}"
  fi
}

# --- Create manifest.json (reproducibility record) ---
manifest="${release_root}/manifest.json"
{
  echo '{'
  echo "  \"package\": \"xezim\","
  echo "  \"version\": \"${VERSION}\","
  echo "  \"platform\": \"${TARGET}\","
  echo "  \"arch\": \"${ARCH}\","
  echo "  \"built_at\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
  echo "  \"rustc\": \"$(rustc --version)\","
  echo "  \"cargo\": \"$(cargo --version)\","
  echo "  \"cargo_lock_hash\": \"$(sha256_file "${SRC_DIR}/Cargo.lock" 2>/dev/null | cut -d' ' -f1 || echo 'unknown')\""
  echo '}'
} > "${manifest}"

# --- Package tarball ---
tarball="xezim-${TARGET}-${VERSION}.tar.gz"
( cd "$(dirname "${release_root}")"
  tar -czf "${OUT_DIR}/${tarball}" "$(basename "${release_root}")"
)

# --- SHA256 companion ---
sha256_file "${OUT_DIR}/${tarball}" > "${OUT_DIR}/${tarball}.sha256"

# --- Copy manifest alongside tarball (per-platform) ---
cp "${manifest}" "${OUT_DIR}/manifest-${TARGET}.json"

echo "[build-release] done: ${OUT_DIR}/${tarball}"
ls -la "${OUT_DIR}/"