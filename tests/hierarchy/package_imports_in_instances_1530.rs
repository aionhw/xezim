//! §23.3.2, §26.3 (#1530): a package name an instance imports is visible in
//! that instance only. The design is flattened into one name space whose bare
//! names are the top-level module's, so an instance's imported parameter or
//! enum member read the top's same-named variable or parameter, and an
//! explicit import inlined for the instance overwrote the top's own
//! declaration. A struct-typed package parameter also registered its member
//! constants (`K.c`) under the bare name, which a module's own struct
//! variable `K` then read without importing anything.
//!
//! Every expectation was cross-checked against the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(src: &str, wants: &[&str]) {
    let o = lines(src);
    for want in wants {
        assert!(o.iter().any(|l| l == want), "missing {want:?} in {o:?}");
    }
}

#[test]
fn imported_package_parameters_do_not_read_top_declarations() {
    expect(
        r#"
package p;
  typedef struct packed {
    logic       a;
    logic       b;
    logic [4:0] c;
  } s_t;
  localparam s_t K = '{a: 1'b0, b: 1'b1, c: 5'd29};
  localparam int N = 7;
endpackage
module sub_wild;
  import p::*;
  logic [4:0] w;
  int n;
  assign w = K.c;
  assign n = N;
endmodule
module sub_explicit;
  import p::K, p::N;
  logic [4:0] w;
  int n;
  assign w = K.c;
  assign n = N;
endmodule
module t;
  typedef struct packed {
    logic       a;
    logic       b;
    logic [4:0] c;
  } s_t;
  s_t K;
  int N;
  sub_wild u1();
  sub_explicit u2();
  initial begin
    K = '{a: 1'b1, b: 1'b0, c: 5'd3};
    N = 3;
    #1 $display("wild w=%b n=%0d explicit w=%b n=%0d top K.c=%b N=%0d", u1.w, u1.n, u2.w, u2.n, K.c, N);
    $display("RESULT %s", (u1.w === 5'd29 && u1.n == 7 && u2.w === 5'd29 && u2.n == 7
                           && K.c === 5'd3 && N == 3) ? "PASS" : "FAIL");
  end
endmodule
"#,
        &[
            "wild w=11101 n=7 explicit w=11101 n=7 top K.c=00011 N=3",
            "RESULT PASS",
        ],
    );
}

#[test]
fn imported_names_stay_in_their_instance() {
    expect(
        r#"
package pa;
  localparam int N = 7;
  typedef enum logic [1:0] {IDLE, BUSY, DONE} st_t;
  int pv = 11;
  localparam logic [3:0] ARR [2] = '{4'd5, 4'd6};
  function automatic int fn(); return 70; endfunction
endpackage
package pb;
  localparam int N = 8;
endpackage
module sa;
  import pa::*;
  int n; logic [N-1:0] v; st_t s; int x; int y; int z;
  assign n = N;
  assign s = BUSY;
  initial begin #1 x = pv; y = ARR[1]; z = fn(); end
endmodule
module sb;
  import pb::N;
  int n;
  assign n = N;
endmodule
module sc;
  import pa::N;
  int n;
  for (genvar i = 0; i < N; i++) begin : g
    int k;
  end
  assign n = $bits(g[N-1].k) + N;
endmodule
module mid;
  int N;
  sa u3();
  initial N = 5;
endmodule
module t;
  localparam int N = 3;
  int BUSY;
  int pv;
  int ARR;
  function automatic int fn(); return 99; endfunction
  sa u1();
  sb u2();
  sc u4();
  mid m();
  initial begin
    BUSY = 42; pv = 1; ARR = 2;
    #2;
    $display("T|sa n=%0d bits=%0d s=%0d x=%0d y=%0d z=%0d", u1.n, $bits(u1.v), u1.s, u1.x, u1.y, u1.z);
    $display("T|sb n=%0d sc n=%0d mid n=%0d top N=%0d BUSY=%0d pv=%0d fn=%0d", u2.n, u4.n, m.u3.n, N, BUSY, pv, fn());
  end
endmodule
"#,
        &[
            "T|sa n=7 bits=7 s=1 x=11 y=6 z=70",
            "T|sb n=8 sc n=39 mid n=7 top N=3 BUSY=42 pv=1 fn=99",
        ],
    );
}

#[test]
fn package_struct_parameter_members_need_an_import() {
    expect(
        r#"
package p;
  typedef struct packed { logic a; logic b; logic [4:0] c; } s_t;
  localparam s_t K = '{a: 1'b0, b: 1'b1, c: 5'd29};
endpackage
module t;
  typedef struct packed { logic a; logic b; logic [4:0] c; } s_t;
  s_t K;
  
  initial begin K = '{a: 1'b1, b: 1'b0, c: 5'd3}; #1 $display("T|K.c=%0d K=%b", K.c, K); end
endmodule
"#,
        &["T|K.c=3 K=1000011"],
    );
}
