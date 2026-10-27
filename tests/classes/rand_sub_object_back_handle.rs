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
