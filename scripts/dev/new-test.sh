#!/usr/bin/env bash
# Scaffold a new integration test in a group and wire it into the group root.
#
#   scripts/dev/new-test.sh scheduling regress_foo
#   cargo test --test scheduling regress_foo
#
# Creates tests/<group>/<name>.rs and appends the #[path] + mod lines to
# tests/<group>.rs. The classic failure is forgetting the mod line — this
# script makes it impossible.
set -uo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $(basename "$0") <group> <snake_case_name>" >&2
  exit 2
fi
group="$1"
name="$2"

case "$group" in
  classes|collections|gates|hierarchy|misc|scheduling|strings|types) ;;
  *) echo "unknown group '$group' (expected one of: classes collections gates hierarchy misc scheduling strings types)" >&2; exit 1 ;;
esac
if ! printf '%s' "$name" | grep -qE '^[a-z][a-z0-9_]*$'; then
  echo "invalid test name '$name' (use snake_case, lowercase start)" >&2
  exit 1
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
root="$repo/tests/$group.rs"
file="$repo/tests/$group/$name.rs"

if [ -e "$file" ]; then
  echo "already exists: $file" >&2
  exit 1
fi

cat >"$file" <<EOF
//! <LRM § + what this guards> — see docs/dev/testing.md for conventions.
use xezim::simulate;

#[test]
fn $name() {
    let sim = simulate(
        r#"
module tb;
  // minimal repro
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    // assert_eq!(u(&sim, "tb.sig"), N, "expected value");
}
EOF

printf '#[path = "%s/%s.rs"]\nmod %s;\n' "$group" "$name" "$name" >>"$root"

echo "created $file"
echo "appended mod line to $root"
echo "run it with: cargo test --test $group $name"
