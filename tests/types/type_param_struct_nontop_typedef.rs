//! §6.20.3, §23.10: a type parameter override names a type in the
//! instantiating scope. When that scope is a module instance below the top,
//! its typedefs (`id_t`) and the struct built from them (`s_t`) outlive its
//! inlining only under instance-scoped keys (`m.id_t`). The override's
//! struct kept a BARE `id_t` member, so once `m`'s bare typedefs were
//! restored the member resolved to nothing and took the 32-bit `int` default:
//! `$bits(a[0].id)` read 32 and `a[1].id` read 32 bits from the id's MSB
//! down (§7.2, §7.4.1). The struct had to be at least 32 bits wide for the
//! wrong layout to fit, which is why narrower repros worked. A typedef of the
//! child with the member type's name captured the member the same way.
//!
//! Expected values are Verilator's (5.046); display order within a time
//! slot is not checked.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn assert_has(src: &str, expected: &[&str]) {
    let got = lines(src);
    for e in expected {
        assert!(got.iter().any(|l| l == e), "missing `{e}` in {got:#?}");
    }
}

/// The reported shape: a packed array of a 36-bit struct whose member is a
/// typedef of the non-top `mid`, read and written through `a[i].id`.
#[test]
fn packed_array_member_of_struct_from_non_top_module() {
    assert_has(
        r#"
module child #(parameter int N = 2, parameter type T = logic)
  (input T [N-1:0] a, output T [N-1:0] b);
  always_comb begin
    b = a;
    b[0].id = 4'hF;
  end
  initial #1 $display("c1: $bits(a[0].id)=%0d a[1].id=%h b[0]=%h b[1]=%h",
                      $bits(a[0].id), a[1].id, b[0], b[1]);
endmodule
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x, y;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
  child #(.N(2), .T(s_t)) c (.a(x), .b(y));
  initial #2 $display("c1: y[0].id=%h y[1].id=%h", y[0].id, y[1].id);
endmodule
module top;
  mid m();
endmodule
"#,
        &[
            "c1: $bits(a[0].id)=4 a[1].id=2 b[0]=f000000ab b[1]=2000000cd",
            "c1: y[0].id=f y[1].id=2",
        ],
    );
}

/// The type passed on through a second type parameter (`.T(U)`).
#[test]
fn type_parameter_passed_through_two_levels() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T [1:0] a, output logic [3:0] o);
  assign o = a[1].id;
  initial #1 $display("c2: child $bits(a[0].id)=%0d a[1].id=%h o=%h", $bits(a[0].id), a[1].id, o);
endmodule
module mid #(parameter type U = logic) (input U [1:0] a);
  child #(.T(U)) c (.a(a), .o());
  initial #1 $display("c2: mid $bits(a[0].id)=%0d a[1].id=%h", $bits(a[0].id), a[1].id);
endmodule
module outer;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
  mid #(.U(s_t)) m (.a(x));
endmodule
module top; outer o(); endmodule
"#,
        &[
            "c2: mid $bits(a[0].id)=4 a[1].id=2",
            "c2: child $bits(a[0].id)=4 a[1].id=2 o=2",
        ],
    );
}

/// §27: typedefs declared in a generate block of the non-top module.
#[test]
fn typedef_in_generate_block_of_non_top_module() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T [1:0] a);
  initial #1 $display("c3: $bits(a[0].id)=%0d a[1].id=%h", $bits(a[0].id), a[1].id);
endmodule
module mid;
  if (1) begin : g
    typedef logic [3:0] id_t;
    typedef struct packed { id_t id; logic [31:0] data; } s_t;
    s_t [1:0] x;
    initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
    child #(.T(s_t)) c (.a(x));
  end
endmodule
module top; mid m(); endmodule
"#,
        &["c3: $bits(a[0].id)=4 a[1].id=2"],
    );
}

/// §25: an interface instance's type parameter bound to the struct.
#[test]
fn interface_type_parameter_bound_in_non_top_module() {
    assert_has(
        r#"
interface bus_if #(parameter type T = logic);
  T [1:0] v;
  logic [3:0] id1;
  assign id1 = v[1].id;
endinterface
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  bus_if #(.T(s_t)) b ();
  initial begin b.v[0] = 36'h1_0000_00AB; b.v[1] = 36'h2_0000_00CD;
    #1 $display("c4: id1=%h v[1].id=%h $bits(b.v[0].id)=%0d", b.id1, b.v[1].id, $bits(b.v[0].id)); end
endmodule
module top; mid m(); endmodule
"#,
        &["c4: id1=2 v[1].id=2 $bits(b.v[0].id)=4"],
    );
}

/// §7.4: unpacked array ports of the struct.
#[test]
fn unpacked_array_ports_of_the_struct() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T a [2], output T b [2]);
  always_comb begin b = a; b[0].id = 4'hF; end
  initial #1 $display("c5: $bits(a[1].id)=%0d a[1].id=%h b[0]=%h b[1].id=%h", $bits(a[1].id), a[1].id, b[0], b[1].id);
endmodule
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t x [2];
  s_t y [2];
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
  child #(.T(s_t)) c (.a(x), .b(y));
endmodule
module top; mid m(); endmodule
"#,
        &["c5: $bits(a[1].id)=4 a[1].id=2 b[0]=f000000ab b[1].id=2"],
    );
}

/// Nonblocking member writes, a continuous assignment and an output port.
#[test]
fn nba_continuous_assign_and_output_port_paths() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input logic clk, input T [1:0] a, output T [1:0] q, output logic [3:0] ca);
  assign ca = a[1].id ^ a[0].id;
  always_ff @(posedge clk) begin
    q <= a;
    q[1].id <= a[0].id;
  end
  initial #3 $display("c6: q[0]=%h q[1]=%h q[1].id=%h ca=%h", q[0], q[1], q[1].id, ca);
endmodule
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x, q;
  logic clk = 0;
  logic [3:0] ca;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; #1 clk = 1; end
  child #(.T(s_t)) c (.clk(clk), .a(x), .q(q), .ca(ca));
  initial #3 $display("c6: mid q[1].id=%h q[1].data=%h ca=%h", q[1].id, q[1].data, ca);
endmodule
module top; mid m(); endmodule
"#,
        &[
            "c6: q[0]=1000000ab q[1]=1000000cd q[1].id=1 ca=3",
            "c6: mid q[1].id=1 q[1].data=000000cd ca=3",
        ],
    );
}

/// §6.20.3: the child's own `id_t` (8 bits) must not capture the member
/// type, which names `mid`'s `id_t` (4 bits).
#[test]
fn child_typedef_of_the_same_name_does_not_capture_the_member() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T [1:0] a);
  typedef logic [7:0] id_t;
  id_t own;
  initial begin own = 8'hA5; #1 $display("c7: $bits(a[0].id)=%0d a[1].id=%h $bits(own)=%0d", $bits(a[0].id), a[1].id, $bits(own)); end
endmodule
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
  child #(.T(s_t)) c (.a(x));
endmodule
module top; mid m(); endmodule
"#,
        &["c7: $bits(a[0].id)=4 a[1].id=2 $bits(own)=8"],
    );
}

/// A `T`-typed local in a process and a function argument of the child.
#[test]
fn type_parameter_locals_and_function_arguments() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T [1:0] a, output logic [3:0] r);
  function automatic logic [3:0] get_id(T s); return s.id; endfunction
  always_comb begin
    T tmp;
    tmp = a[1];
    tmp.id = tmp.id + 4'd1;
    r = tmp.id;
  end
  initial #1 $display("c8: r=%h get_id=%h $bits(a[0].id)=%0d", r, get_id(a[1]), $bits(a[0].id));
endmodule
module mid;
  typedef logic [3:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x;
  logic [3:0] r;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD; end
  child #(.T(s_t)) c (.a(x), .r(r));
endmodule
module top; mid m(); endmodule
"#,
        &["c8: r=3 get_id=2 $bits(a[0].id)=4"],
    );
}

/// §23.10: two instances of the non-top module whose member typedef has
/// different widths, each with its own layout.
#[test]
fn per_instance_member_typedef_widths() {
    assert_has(
        r#"
module child #(parameter type T = logic) (input T [1:0] a);
  initial #1 $display("c9: %m $bits(a[0].id)=%0d a[1].id=%h", $bits(a[0].id), a[1].id);
endmodule
module mid #(parameter int W = 4);
  typedef logic [W-1:0] id_t;
  typedef struct packed { id_t id; logic [31:0] data; } s_t;
  s_t [1:0] x;
  initial begin x = '0; x[1].id = '1; end
  child #(.T(s_t)) c (.a(x));
endmodule
module top; mid #(4) m4(); mid #(8) m8(); endmodule
"#,
        &[
            "c9: top.m4.c $bits(a[0].id)=4 a[1].id=f",
            "c9: top.m8.c $bits(a[0].id)=8 a[1].id=ff",
        ],
    );
}

#[test]
fn child_type_formals_do_not_capture_sibling_actuals() {
    assert_has(
        r#"
module child #(parameter type A = int, parameter type B = int) ();
  A first;
  B second;
  initial #1 $display("scope %m %0d %0d", $bits(first), $bits(second));
endmodule
module parent #(parameter type A = logic) ();
  child #(.A(byte), .B(A)) named();
  child #(.B(A), .A(byte)) reversed();
  child #(byte, A) ordered();
endmodule
module top; parent #(.A(logic [4:0])) p(); endmodule
"#,
        &[
            "scope top.p.named 8 5",
            "scope top.p.reversed 8 5",
            "scope top.p.ordered 8 5",
        ],
    );
}
