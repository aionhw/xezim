//! §18.5.9: a rand sub-object's constraint that reads the enclosing object's
//! rand variable through a handle back to it (`p.x`) is solved jointly with
//! the enclosing object. The sub-object was solved against the old value of
//! `x`, so randomize() failed whenever that value broke the parent's own
//! constraints, though a solution exists.
#[test]
fn sub_object_reads_parent_through_back_handle() {
    let result = xezim::simulate(
        r#"
typedef class sub_c;
class cfg_c;
  rand sub_c sub;
  rand bit [7:0] x;
  constraint k { x inside {[1:200]}; }
  function new(); sub = new(); sub.p = this; endfunction
endclass
class sub_c;
  cfg_c p;
  rand bit [7:0] y;
  constraint s { y == p.x + 8'd1; }
endclass
module t;
  initial begin
    cfg_c c = new();
    int ok = 0, bad = 0;
    repeat (20) if (c.randomize()) begin
      ok++;
      bad += !(c.x inside {[1:200]}) || c.sub.y != c.x + 8'd1;
    end
    $display("ok %0d bad %0d", ok, bad);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 20 bad 0"]);
}

/// The same read of an enclosing object's rand array element (`p.x[0]`) is
/// solved with the enclosing object.
#[test]
fn sub_object_reads_parent_array_through_back_handle() {
    let result = xezim::simulate(
        r#"
typedef class sub_c;
class cfg_c;
  rand sub_c sub;
  rand bit [7:0] x[1];
  constraint k { x[0] == 1; }
  function new(); sub = new(); sub.p = this; endfunction
endclass
class sub_c;
  cfg_c p;
  rand bit [7:0] y;
  constraint s { y == p.x[0]; y == 8'd1; }
endclass
module t;
  initial begin
    cfg_c c = new();
    int ok = 0, bad = 0;
    c.x[0] = 5;
    repeat (20) if (c.randomize()) begin
      ok++;
      bad += c.x[0] != 1 || c.sub.y != 1;
    end
    $display("ok %0d bad %0d", ok, bad);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 20 bad 0"]);
}

/// The read may sit in a grandchild that reaches the enclosing object through
/// the child (`owner.p.x`); the child itself reads nothing.
#[test]
fn grandchild_reads_parent_through_back_handles() {
    let result = xezim::simulate(
        r#"
typedef class P;
typedef class G;
class C;
  P p;
  rand G g;
endclass
class G;
  C owner;
  rand bit [7:0] y;
  constraint k { y == owner.p.x + 1; }
endclass
class P;
  rand bit [7:0] x;
  rand C c;
  constraint k { x inside {[10:20]}; }
  function new();
    c = new; c.p = this; c.g = new; c.g.owner = c;
  endfunction
endclass
module t;
  initial begin
    P o = new;
    int ok = 0, good = 0;
    repeat (20) begin
      o.x = 200;
      if (o.randomize()) begin
        ok++;
        if (o.x inside {[10:20]} && o.c.g.y == o.x + 1) good++;
      end
    end
    $display("ok %0d good %0d", ok, good);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 20 good 20"]);
}

/// A sub-object that only stores the enclosing object's handle and reads
/// nothing through it stays out of the joint solve, so its `randc` member
/// still cycles and the enclosing object still randomizes.
#[test]
fn independent_randc_sub_object_with_owner_handle_stays_out() {
    let result = xezim::simulate(
        r#"
typedef class cfg_c;
class tag_c;
  cfg_c owner;
  randc bit [1:0] r;
endclass
class part_c;
  rand bit [7:0] v;
endclass
class cfg_c;
  rand tag_c tag;
  rand part_c part;
  rand bit [7:0] x;
  constraint k { x inside {[10:20]}; part.v == x + 8'd1; }
  function new(); tag = new(); tag.owner = this; part = new(); endfunction
endclass
module t;
  initial begin
    cfg_c c = new();
    int ok = 0, bad = 0, seen = 0;
    repeat (4) begin
      c.x = 200;
      if (c.randomize()) begin
        ok++;
        bad += !(c.x inside {[10:20]}) || c.part.v != c.x + 8'd1;
        seen |= 1 << c.tag.r;
      end
    end
    $display("ok %0d bad %0d seen %0d", ok, bad, seen);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 4 bad 0 seen 15"]);
}
