//! IEEE 1800-2023 §6.8 Table 6-7: a 4-state variable starts at x and a net
//! with no driver at z; 2-state variables start at 0. Pins what `$display`
//! shows at time 0 for every kind of declaration, so no later change turns an
//! x into a 0. Expected lines are the reference simulator's output.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use std::path::PathBuf;
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_time0_x_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `src` with top `top`, return its `T|` lines.
fn run(tag: &str, src: &str, top: &str) -> Vec<String> {
    let d = scratch(tag);
    std::fs::write(d.join("tb.sv"), src).unwrap();
    let out = Command::new(xezim())
        .current_dir(&d)
        .args(["-s", top, "tb.sv"])
        .env_remove("XEZIM_INIT_REG")
        .env_remove("XEZIM_INIT_ZERO")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&d);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

const DESIGN: &str = r#"
module sub; logic [3:0] s; endmodule
package p; logic [3:0] pv; endpackage
typedef struct { logic [3:0] f; } st_t;
class C; logic [3:0] m; logic [3:0] arr[2]; endclass
module top;
  logic [3:0] a;
  logic b;
  logic [3:0] arr [2];
  logic [3:0] q [$];
  logic [3:0] da [];
  st_t st;
  reg [3:0] r;
  wire [3:0] w;
  integer i;
  int ii;
  sub u();
  C c;
  logic [7:0] ff;
  always @(posedge b) ff <= a;
  function void f(); logic [3:0] lf; $display("T|func local lf=%b", lf); endfunction
  task automatic tk(); logic [3:0] lt; $display("T|task auto local lt=%b", lt); endtask
  initial begin
    logic [3:0] li;
    $display("T|a=%b b=%b arr0=%b st.f=%b r=%b w=%b i=%b ii=%0d u.s=%b p::pv=%b ff=%b",
             a, b, arr[0], st.f, r, w, i, ii, u.s, p::pv, ff);
    $display("T|initial local li=%b", li);
    $display("T|%%d a=%0d %%h a=%h %%p arr=%p", a, a, arr);
    f(); tk();
    c = new; $display("T|class m=%b arr0=%b", c.m, c.arr[0]);
    da = new[1]; $display("T|da0=%b", da[0]);
    #1 $display("T|t1 a=%b ff=%b", a, ff);
  end
endmodule
"#;

#[test]
fn every_declaration_kind_starts_unknown_at_time_zero() {
    let got = run("kinds", DESIGN, "top");
    let want = [
        "T|a=xxxx b=x arr0=xxxx st.f=xxxx r=xxxx w=zzzz i=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx ii=0 u.s=xxxx p::pv=xxxx ff=xxxxxxxx",
        "T|initial local li=xxxx",
        "T|%d a=x %h a=x %p arr='{'hX, 'hX}",
        "T|func local lf=xxxx",
        "T|task auto local lt=xxxx",
        "T|class m=xxxx arr0=xxxx",
        "T|da0=xxxx",
        "T|t1 a=xxxx ff=xxxxxxxx",
    ];
    assert_eq!(got, want);
}
