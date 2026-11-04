//! §6.21, §13.3.1: automatic locals of two concurrent class tasks that share a
//! name but not a type. Each declaration registered its name's enum typedef,
//! struct members, packed layout and class in tables keyed by the bare name,
//! so the task that declared last displaced the other's: `c.num()` counted
//! the other enum, `%p` printed the other struct's members and `w.f[3]`
//! used the other packed layout. Expected values are the reference
//! simulator's.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef enum int { RED, GREEN, BLUE } color_e;
  typedef enum int { P0, P1, P2, P3 } fruit_e;
  typedef struct { int a; string s; } uA_t;
  typedef struct { int x; int y; } uB_t;
  typedef struct packed { logic [7:0][3:0] f; logic [3:0] pad; } wide_t;
  typedef struct packed { logic [15:0] f; logic [19:0] other; } narrow_t;
  typedef struct packed { logic [7:0] addr; logic [7:0] id; } req_t;
  class H; int id = 9; endclass
  class V;
    task t25(); color_e c; c = GREEN; #20; $display("T|s25 %s %0d %s", c.name(), c.num(), c.next().name()); endtask
    task t26(); uA_t u; u.a = 7; u.s = "hi"; #20; $display("T|s26 %p", u); endtask
    task t27(); wide_t w; w = '0; w.f[3] = 4'hA; w.f[0] = 4'h5; #20; $display("T|s27 %h %h", w.f[3], w.f[0]); endtask
    task t23(); req_t r; #20; r.addr = 8'h12; r.id = 8'h34; $display("T|s23 %h %h", r.addr, r.id); endtask
    task t30(); event e; fork begin @(e); $display("T|s30 woke at %0t", $time); end join_none #40 -> e; #1; endtask
  endclass
  class I;
    task t25(); fruit_e c; forever #13 c = P1; endtask
    task t26(); uB_t u; u.x = 1; u.y = 2; forever #13; endtask
    task t27(); narrow_t w; w = '0; forever #13; endtask
    task t23(); H r; forever #13; endtask
    task t30(); event e; #10 -> e; forever #13; endtask
  endclass
  V v = new; I i = new;
  initial begin
    fork
      begin #5 i.t25(); end begin #5 i.t26(); end begin #5 i.t27(); end begin #5 i.t23(); end begin #5 i.t30(); end
      v.t25(); v.t26(); v.t27(); v.t23(); v.t30();
    join_any
    #50 $finish;
  end
endmodule
"#;

#[test]
fn concurrent_same_named_locals_keep_their_own_types() {
    let lines: Vec<String> = simulate(SRC, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|s25 GREEN 3 BLUE",
            r#"T|s26 '{a:7, s:"hi"}"#,
            "T|s27 a 5",
            "T|s23 12 34",
            "T|s30 woke at 40",
        ]
    );
}
