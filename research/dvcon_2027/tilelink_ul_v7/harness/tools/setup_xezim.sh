#!/usr/bin/env bash
# Read-only pinned checkout. Does not change any existing Xezim worktree.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SHA=80f00d354fc68c436569b12b6d4e175ef105a652
CORE=837725742ba9aaa3b3408f770b288cf85f020ef9
DEST="${XEZIM_TILELINK_DEPS:-$HERE/.deps}/xezim"
mkdir -p "$(dirname "$DEST")"
if [ -e "$DEST" ]; then
  echo "Refusing to overwrite existing checkout $DEST" >&2
  exit 2
fi
git clone --filter=blob:none https://github.com/aionhw/xezim.git "$DEST"
git -C "$DEST" checkout --detach "$SHA"
ACTUAL="$(git -C "$DEST" rev-parse HEAD)"
[ "$ACTUAL" = "$SHA" ] || { echo 'Commit mismatch' >&2; exit 3; }
grep -q "rev = \"$CORE\"" "$DEST/Cargo.toml" || {
  echo 'Pinned core revision changed; STOP' >&2; exit 4;
}
if [ ! -f "$DEST/Cargo.lock" ]; then
  echo 'Upstream does not commit Cargo.lock. Resolving a NEW local lockfile before freeze.'
  (cd "$DEST" && cargo generate-lockfile)
fi
sha256sum "$DEST/Cargo.lock" | tee "$HERE/config/cargo_lock_sha256.txt"
(cd "$DEST" && cargo build --release --locked --bin xezim)
"$DEST/target/release/xezim" -V
sha256sum "$DEST/target/release/xezim" | tee "$HERE/config/binary_sha256.txt"
echo "Pinned at $SHA. Build logs and binary hashes are local; never replace with a moving main."
