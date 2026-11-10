//! §7.2, §27, §21.2.1.7: an unpacked array of unpacked structs declared in a
//! generate block stores each element member-wise, like one declared in the
//! module, and `%p` renders an element, a nested member and a whole array of
//! a generate block's variable from its declared type, also through a path
//! from outside the block. Every expected line is the reference simulator's.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn a_generate_blocks_struct_array_elements_hold_their_members() {
    let src = r#"
module top;
  typedef struct { int f; int g; } s_t;
  if (1) begin : g
    s_t q [3];
    int k = 2;
    initial begin
      q[0].f = 10; q[2] = '{5, 6};
      q[1].g = 7;
      #1 $display("T|%0d %0d %0d %0d", q[k].f, q[2].g, q[1].g, q[0].f);
    end
  end
  for (genvar i = 0; i < 2; i++) begin : l
    s_t t [2];
    int j = 1;
    initial begin
      t[0].f = 100 + i; t[1].f = 200 + i;
      #2 $display("T|l[%0d] %0d %0d", i, t[j].f, t[i].f);
    end
  end
  initial #3 $display("T|top %0d %0d %0d", g.q[2].f, g.q[1].g, l[1].t[0].f);
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|5 6 7 10",
            "T|l[0] 200 100",
            "T|l[1] 201 201",
            "T|top 5 7 101"
        ]
    );
}

#[test]
fn p_renders_elements_and_members_of_a_generate_blocks_structs() {
    let src = r#"
module top;
  typedef struct { int f; int g; } s_t;
  typedef struct { s_t in; int z; } o_t;
  if (1) begin : g
    s_t q [3];
    o_t o;
    o_t oa [2];
    initial begin
      q[2] = '{5, 6}; o.in = '{1, 2}; o.z = 3; oa[1].in.g = 9;
      #1 $display("T|%p %p %p %p %p", q[2], o, o.in, oa[1], oa[1].in);
    end
  end
  for (genvar i = 0; i < 2; i++) begin : l
    s_t t [2];
    initial begin t[1] = '{i, 7}; #2 $display("T|l[%0d] %p %p", i, t[1], t); end
  end
  initial #3 $display("T|top %p %p %p", g.q[2], g.o.in, l[1].t[1]);
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|'{f:5, g:6} '{in:'{f:1, g:2}, z:3} '{f:1, g:2} '{in:'{f:0, g:9}, z:0} '{f:0, g:9}",
            "T|l[0] '{f:0, g:7} '{'{f:0, g:0}, '{f:0, g:7}}",
            "T|l[1] '{f:1, g:7} '{'{f:0, g:0}, '{f:1, g:7}}",
            "T|top '{f:5, g:6} '{f:1, g:2} '{f:1, g:7}",
        ]
    );
}
