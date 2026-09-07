//! §6.16/§7.4: a FIXED array of `string` declared inside a block or
//! subroutine — its `'{…}` / `'{default:…}` initializer and its unset
//! elements.
//!
//! The local fixed-array path seeded every element as a zero Value and never
//! marked the array as string-typed, so an unset element rendered under `%s`
//! as a run of blanks (a width's worth of NULs) and the declaration's pattern
//! spread had no string-typed element to land in: `string nm[3] = '{"a","b",
//! "c"}` in an `initial` block printed nothing but padding under `%-18s`,
//! which is how it first surfaced. Locals now mirror the string-queue
//! convention — the array name joins `string_signals` and elements seed as
//! the empty string — so the pattern applies and `%s`/`.len()` see strings.
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn local_string_array_pattern_init_and_unset_elements() {
    let o = out(r#"
module tb;
  initial begin
    string p [3] = '{"aa", "bb", "cc"};
    string q [3];
    string r [3] = '{default: "zz"};
    q[1] = "qq";
    $display("P1=[%s] len=%0d P2=[%s]", p[1], p[1].len(), p[2]);
    $display("Q1=[%s] len=%0d Q0=[%s] len=%0d", q[1], q[1].len(), q[0], q[0].len());
    $display("R1=[%s]", r[1]);
    $display("P1pad=[%-6s]", p[1]);
    for (int i = 0; i < 3; i++) $display("A%02d %-6s|", i, p[i]);   // the reporting shape
  end
endmodule
"#);
    for expect in [
        "P1=[bb] len=2 P2=[cc]",
        "Q1=[qq] len=2 Q0=[] len=0",
        "R1=[zz]",
        "P1pad=[bb    ]",
        "A 1 bb    |",
    ] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}

#[test]
fn string_array_locals_in_task_frames_two_dims_and_calls() {
    // A 2-D string local's pattern went through the generic leaf writer,
    // which resized every element to its nominal width — NUL padding that
    // printed as blanks before each value. String leaves now store as-is.
    let o = out(r#"
module tb;
  function automatic int total(string a [2]); return a[0].len() + a[1].len(); endfunction
  task automatic t();
    string s [2] = '{"task", "frame"};
    string m [2][2] = '{'{"a", "bb"}, '{"ccc", "dddd"}};
    int n = 0;
    foreach (s[i]) n += s[i].len();
    $display("T s1=[%s] n=%0d m11=[%s] m01=[%s] fn=%0d", s[1], n, m[1][1], m[0][1], total(s));
    $display("P %p", s);
  endtask
  initial t();
endmodule
"#);
    for expect in ["T s1=[frame] n=9 m11=[dddd] m01=[bb] fn=9", "P '{\"task\", \"frame\"}"] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}

