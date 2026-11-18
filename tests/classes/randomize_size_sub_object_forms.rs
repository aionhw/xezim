//! §18.5.8.1, §18.5.9, §18.8, §18.7: the size of a dynamic array or queue
//! that a constraint ties to fields of `rand` sub-objects is solved with
//! them in every form: a nested sub-object (`h.in.n`), a queue, a `foreach`
//! that reads a sub-object field, `solve ... before`, an inline `with`, the
//! sub-object or one of its fields switched off with `rand_mode(0)` (the size
//! follows the held value), a sub-object that is not `rand` (its fields are
//! state), and an unsatisfiable inline size (0, values left as they were).
//! The counts are the reference simulator's (#1259).

use xezim::simulate;

#[test]
fn sizes_over_sub_object_fields_in_every_form() {
    let src = r#"
class inner_c;
  rand bit [7:0] n;
  constraint c { n inside {[5:200]}; }
endclass
class hdr;
  rand bit [15:0] len;
  rand bit [7:0] fill;
  rand inner_c in;
  constraint len_c { len inside {[20:576]}; }
  function new(); in = new(); endfunction
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  rand bit [7:0] q[$];
  rand bit [7:0] nest[];
  constraint pad_c { pad.size() == 46 - h.len; foreach (pad[i]) pad[i] == h.fill; }
  constraint q_c { q.size() == h.len - 18; }
  constraint nest_c { nest.size() == 10 - h.in.n; }
  function new(); h = new(); endfunction
endclass
class pkt_sb;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c { solve h.len before pad; pad.size() == 46 - h.len; }
  function new(); h = new(); endfunction
endclass
class pkt_nr;           // non-rand sub-object: its fields are state
  hdr h;
  rand bit [7:0] pad[];
  constraint pad_c { pad.size() == 46 - h.len; }
  function new(); h = new(); h.len = 30; endfunction
endclass
module top;
  initial begin
    pkt p = new();
    pkt_sb s = new();
    pkt_nr r = new();
    int ok, bad;
    ok = 0; bad = 0;
    repeat (20) begin
      if (p.randomize()) begin
        ok++;
        if (p.h.len < 20 || p.h.len > 46 || p.pad.size() != 46 - p.h.len || p.q.size() != p.h.len - 18
            || p.h.in.n < 5 || p.h.in.n > 10 || p.nest.size() != 10 - p.h.in.n) bad++;
        foreach (p.pad[i]) if (p.pad[i] != p.h.fill) bad++;
      end
    end
    $display("T|plain ok=%0d bad=%0d", ok, bad);
    ok = 0; bad = 0;
    repeat (20) begin
      if (p.randomize() with { pad.size() == 3; }) begin
        ok++;
        if (p.h.len != 43 || p.pad.size() != 3 || p.q.size() != 25) bad++;
      end
    end
    $display("T|with ok=%0d bad=%0d", ok, bad);
    ok = 0; bad = 0;
    p.h.rand_mode(0);
    p.h.len = 40; p.h.in.n = 7;
    repeat (5) begin
      if (p.randomize()) begin
        ok++;
        if (p.h.len != 40 || p.pad.size() != 6 || p.q.size() != 22 || p.nest.size() != 3) bad++;
      end
    end
    $display("T|rand_mode0 ok=%0d bad=%0d", ok, bad);
    p.h.rand_mode(1);
    p.h.len.rand_mode(0);
    p.h.len = 44;
    ok = 0; bad = 0;
    repeat (5) begin
      if (p.randomize()) begin
        ok++;
        if (p.h.len != 44 || p.pad.size() != 2 || p.q.size() != 26) bad++;
      end
    end
    $display("T|len_rand_mode0 ok=%0d bad=%0d", ok, bad);
    p.h.len.rand_mode(1);
    ok = 0; bad = 0;
    repeat (20) begin
      if (s.randomize()) begin
        ok++;
        if (s.h.len < 20 || s.h.len > 46 || s.pad.size() != 46 - s.h.len) bad++;
      end
    end
    $display("T|solve_before ok=%0d bad=%0d", ok, bad);
    ok = 0; bad = 0;
    repeat (5) begin
      if (r.randomize()) begin
        ok++;
        if (r.h.len != 30 || r.pad.size() != 16) bad++;
      end
    end
    $display("T|nonrand ok=%0d bad=%0d", ok, bad);
    // unsatisfiable: values are left unchanged
    p.pad = new[2]; p.pad[0] = 1; p.pad[1] = 2; p.h.len = 25;
    ok = p.randomize() with { pad.size() == 50; };
    $display("T|unsat ok=%0d len=%0d size=%0d p0=%0d", ok, p.h.len, p.pad.size(), p.pad[0]);
    $finish;
  end
endmodule
"#;
    let lines: Vec<String> = simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|plain ok=20 bad=0",
            "T|with ok=20 bad=0",
            "T|rand_mode0 ok=5 bad=0",
            "T|len_rand_mode0 ok=5 bad=0",
            "T|solve_before ok=20 bad=0",
            "T|nonrand ok=5 bad=0",
            "T|unsat ok=0 len=25 size=2 p0=1",
        ]
    );
}
