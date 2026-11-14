//! §10.8, §10.9, §11.4.5, §11.4.6 (#1043): a bare assignment pattern compared
//! with a PACKED operand takes the operand's type and is one packed value of
//! it, so the comparison is bit for bit: an x/z bit leaves `==` x, and the
//! operand's type comes from wherever it is declared — a class property (also
//! through implicit `this`), a function result, a packed element of an
//! unpacked array, a nested or arrayed packed struct, a plain vector, a
//! parenthesized operand or a concurrent assertion. Expected values are from
//! the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
}

#[test]
fn packed_operand_forms_take_the_pattern_type() {
    let msgs = lines(
        r#"
module top;
  typedef logic [2:0] u3_t;
  typedef struct packed { u3_t lo; u3_t hi; } ps_t;
  typedef struct packed { ps_t a; ps_t b; } nest_t;
  u3_t [0:3] pa, px;
  ps_t ps, psx;
  nest_t nst;
  ps_t [0:1] aps;
  ps_t uaps [0:1];
  u3_t [0:3] upe [0:1];
  logic [0:1][0:3][2:0] m2;
  logic [7:0] plain;
  bit [3:0][1:0] b2;
  class C;
    u3_t [0:3] cp;
    ps_t cs;
    function bit eq_this(); return cp == '{2,2,2,2}; endfunction
    function bit eq_cs(); return cs == '{lo:1, hi:2}; endfunction
  endclass
  C c;
  function automatic u3_t [0:3] f_ret(); return '{2,2,2,2}; endfunction
  function automatic ps_t f_ps(); return '{lo:1, hi:2}; endfunction
  function automatic bit f_formal(u3_t [0:3] x); return x == '{2,2,2,2}; endfunction
  int n;
  initial begin
    pa = '{2,2,2,2}; px = '{2,2,'x,2};
    ps = '{lo:1, hi:2}; psx = '{lo:'x, hi:2};
    nst = '{a:'{lo:2,hi:2}, b:'{lo:2,hi:2}};
    aps = '{'{lo:1,hi:2}, '{lo:3,hi:4}};
    uaps = '{'{lo:1,hi:2}, '{lo:3,hi:4}};
    upe = '{'{2,2,2,2}, '{2,2,2,2}};
    m2 = '{'{2,2,2,2}, '{2,2,2,2}};
    plain = 8'b1010_0101;
    b2 = '{1,2,3,0};
    c = new; c.cp = '{2,2,2,2}; c.cs = '{lo:1, hi:2};
    $display("X1 %b %b %b %b", pa == '{2,2,'x,2}, pa != '{2,2,'x,2}, pa === '{2,2,'x,2}, pa !== '{2,2,'x,2});
    $display("X2 %b %b %b %b", px == '{2,2,2,2}, px != '{2,2,2,2}, px === '{2,2,'x,2}, px == '{3,2,2,2});
    $display("X3 %b %b %b", psx == '{1,2}, psx != '{1,3}, psx === '{'x,2});
    $display("S1 %b %b %b", f_ret() == '{2,2,2,2}, f_ps() == '{lo:1, hi:2}, f_formal(pa));
    $display("S2 %b %b %b %b", c.cp == '{2,2,2,2}, c.cs == '{1,2}, c.eq_this(), c.eq_cs());
    $display("S3 %b %b", aps[1] == '{lo:3, hi:4}, aps == '{'{1,2},'{3,4}});
    $display("S4 %b %b", uaps[1] == '{lo:3, hi:4}, uaps == '{'{1,2},'{3,4}});
    $display("S5 %b %b", upe[0] == '{2,2,2,2}, upe == '{'{2,2,2,2}, '{2,2,2,2}});
    $display("S6 %b %b", plain == '{1,0,1,0,0,1,0,1}, plain == '{default:1});
    $display("S7 %b %b", b2 == '{1,2,3,0}, b2 == '{3:1, default:0});
    $display("S8 %b %b", ps == '{u3_t:2}, ps == '{lo:1, default:2});
    $display("S9 %b %b %b", (pa) == ('{2,2,2,2}), m2 == '{2{2}}, m2 == '{default:2});
    $display("S10 %b %b", nst == '{a:'{lo:2,hi:2}, b:'{lo:2,hi:2}}, pa ==? '{2,'x,2,2});
    n = 0;
    foreach (upe[i]) if (upe[i] == '{2,2,2,2}) n++;
    $display("S11 %0d", n);
  end
endmodule
"#,
    );
    expect(
        &msgs,
        &[
            "X1 x x 0 1",
            "X2 x x 1 0",
            "X3 x 1 1",
            "S1 1 1 1",
            "S2 1 1 1 1",
            "S3 1 1",
            "S4 1 1",
            "S5 1 1",
            "S6 1 0",
            "S7 1 0",
            "S8 0 1",
            "S9 1 0 1",
            "S10 1 1",
            "S11 2",
        ],
    );
}

#[test]
fn packed_pattern_compare_in_an_assertion_and_a_continuous_assignment() {
    let msgs = lines(
        r#"
module top;
  typedef logic [2:0] u3_t;
  u3_t [0:3] v;
  u3_t ua [0:3];
  logic clk = 0, w1, w2;
  int nf = 0, np = 0;
  assign w1 = (v == '{2,2,2,2});
  assign w2 = (ua == '{2,2,2,2});
  a_cmp: assert property (@(posedge clk) (v == '{2,2,2,2})) np++; else nf++;
  a_un: assert property (@(posedge clk) (ua == '{2,2,2,2})) np++; else nf++;
  always #1 clk = ~clk;
  initial begin
    v = '{2,2,2,2}; ua = '{2,2,2,2};
    repeat (3) @(posedge clk);
    #0.5 $display("A1 nf=%0d np=%0d w1=%b w2=%b", nf, np, w1, w2);
    $finish;
  end
endmodule
"#,
    );
    expect(&msgs, &["A1 nf=0 np=6 w1=1 w2=1"]);
}
