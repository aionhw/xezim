#!/usr/bin/env bash
# Clone every repo needed to run the XuanTie-C910 hello_world test on xezim
# with XTrace, then build the xezim simulator.
#
# Layout produced (all three MUST stay siblings — xezim's Cargo.toml refers
# to ../xezim-core and ../xezim-core/xezim-parser by relative path):
#
#   $DEPS_DIR/xezim/        aionhw/xezim        @ main   the simulator
#   $DEPS_DIR/xezim-core/   aionhw/xezim-core   @ main   engine + xezim-parser
#   $DEPS_DIR/rtlmeter/     verilator/rtlmeter  @ main   XuanTie-C910 RTL + tests
#
# The C910 RTL and the hello test image (inst.pat / data.pat) are committed
# directly inside the rtlmeter repo under designs/XuanTie-C910/, so no
# separate openc910 checkout is needed.
#
# Usage:
#   ./setup.sh                       # clone into ./deps and build
#   DEPS_DIR=/scratch/x ./setup.sh   # clone elsewhere
#   XEZIM_URL=... XEZIM_REF=... ./setup.sh
#
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null && pwd -P)"
DEPS_DIR="${DEPS_DIR:-${SCRIPT_DIR}/deps}"

# ---- Repo URLs / refs (override via env for forks or different remotes) ------
# The xezim repos default to the SSH host aliases used in this environment
# (github-xezim / github-xezim-core). Override XEZIM_URL / XEZIM_CORE_URL
# with a plain "git@github.com:..." or "https://github.com/..." form if you
# do not have those aliases in ~/.ssh/config.
XEZIM_URL="${XEZIM_URL:-https://github.com/aionhw/xezim}"
XEZIM_CORE_URL="${XEZIM_CORE_URL:-https://github.com/aionhw/xezim-core}"
RTLMETER_URL="${RTLMETER_URL:-https://github.com/verilator/rtlmeter}"

XEZIM_REF="${XEZIM_REF:-main}"
XEZIM_CORE_REF="${XEZIM_CORE_REF:-main}"
RTLMETER_REF="${RTLMETER_REF:-main}"

# ---- Tool checks -------------------------------------------------------------
command -v git   >/dev/null || { echo "git not found in PATH"   >&2; exit 1; }
command -v cargo >/dev/null || { echo "cargo (Rust) not found in PATH" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 not found in PATH" >&2; exit 1; }
python3 -c 'import yaml' 2>/dev/null || {
  echo "python3 'yaml' module missing — install with: pip install pyyaml" >&2; exit 1; }

# ---- Clone / update ----------------------------------------------------------
clone_or_update () {
  local url="$1" dir="$2" ref="$3"
  if [[ -d "${dir}/.git" ]]; then
    echo "==> ${dir##*/}: already present — fetching"
    git -C "${dir}" fetch --quiet origin
  else
    echo "==> ${dir##*/}: cloning ${url}"
    git clone --quiet "${url}" "${dir}"
  fi
  git -C "${dir}" checkout --quiet "${ref}"
  # Fast-forward if the ref is a branch; ignore if it is a detached tag/sha.
  git -C "${dir}" merge --ff-only --quiet "origin/${ref}" 2>/dev/null || true
  echo "    ${dir##*/} @ $(git -C "${dir}" rev-parse --short HEAD) (${ref})"
}

mkdir -p "${DEPS_DIR}"
clone_or_update "${XEZIM_URL}"      "${DEPS_DIR}/xezim"      "${XEZIM_REF}"
clone_or_update "${XEZIM_CORE_URL}" "${DEPS_DIR}/xezim-core" "${XEZIM_CORE_REF}"
clone_or_update "${RTLMETER_URL}"   "${DEPS_DIR}/rtlmeter"   "${RTLMETER_REF}"

# ---- Build xezim -------------------------------------------------------------
# A current build is required: an old xezim mishandles XTrace's per-cycle
# sampling vs. the NBA region and the hello test ends in TEST FAILED.
echo "==> building xezim (cargo build --release) — this takes a few minutes"
( cd "${DEPS_DIR}/xezim" && cargo build --release )

XEZIM_BIN="${DEPS_DIR}/xezim/target/release/xezim"
[[ -x "${XEZIM_BIN}" ]] || { echo "build failed: ${XEZIM_BIN} missing" >&2; exit 1; }

# ---- Note on the one runtime modification -----------------------------------
# No source patch is applied to any cloned repo. The single modification this
# flow needs is a per-run *workaround file*: an empty __rtlmeter_top_include.vh
# staged into the work dir. run_c910_hello_xtrace.sh creates it automatically;
# see README.md ("The __rtlmeter stub") for why.

echo
echo "==> setup complete"
echo "    xezim binary : ${XEZIM_BIN}"
echo "    C910 design  : ${DEPS_DIR}/rtlmeter/designs/XuanTie-C910"
echo
echo "Next:  ./run_c910_hello.sh"
