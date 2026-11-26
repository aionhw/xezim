//! Issue #1302: aliased nets of a user-defined nettype name one physical
//! net (§10.11, §6.6.7), so all drivers across all aliased names, port
//! connections, unpacked arrays, generate blocks, and per-driver delays must
//! resolve together once, and unresolved nettypes permit at most one driver
//! across the aliased component. Expected values were checked against reference
//! simulators.

#[test]
fn issue_1302_nettype_alias_resolution() {
    const SV: &str = r#"
package p;
  typedef struct { real value; int count; } S;
  typedef struct { int a[2]; } SA;

  function automatic real sum_r(input real drivers[]);
    real total = 0.0;
    foreach (drivers[i]) total += drivers[i];
    return total;
  endfunction

  function automatic S sum_s(input S drivers[]);
    S res;
    res.value = 0.0;
    res.count = 0;
    foreach (drivers[i]) begin
      res.value += drivers[i].value;
      res.count += drivers[i].count;
    end
    return res;
  endfunction

  function automatic SA sum_sa(input SA drivers[]);
    SA res;
    res.a[0] = 0;
    res.a[1] = 0;
    foreach (drivers[i]) begin
      res.a[0] += drivers[i].a[0];
      res.a[1] += drivers[i].a[1];
    end
    return res;
  endfunction

  nettype real N with sum_r;
  nettype S NS with sum_s;
  nettype SA NSA with sum_sa;
  nettype real UR;
  nettype S US;
endpackage

module sub_inout(inout p::N x, inout p::N y);
  alias x = y;
endmodule

module sub_loc(inout p::N x, output real r);
  p::N loc;
  alias x = loc;
  assign loc = 2.5;
  assign r = loc;
endmodule

module sub_priv(output real r);
  p::N a, b;
  alias a = b;
  assign a = 1.25;
  assign b = 2.5;
  assign r = b;
endmodule

module drv_r(inout p::N p, input real v);
  assign p = v;
endmodule

module drv_s(inout p::NS p, input real v, input int c);
  assign p = '{v, c};
endmodule

module child_arr_elt(inout p::NS p);
  p::NS loc;
  alias p = loc;
  assign loc = '{1.5, 4};
endmodule

module sub_wor(inout wor w1, inout wor w2);
  alias w1 = w2;
  assign w1 = 1'b1;
endmodule

module top;
  import p::*;

  N a, b;
  alias a = b;
  assign a = 1.25;
  assign b = 2.5;

  NS sa, sb;
  alias sa = sb;
  assign sa = '{value: 1.25, count: 3};
  assign sb = '{value: 2.5, count: 7};

  N c1, c2, c3;
  alias c1 = c2 = c3;
  assign c1 = 1.25;
  assign c2 = 2.5;
  assign c3 = 5.0;

  N t1, t2, t3;
  alias t1 = t2;
  alias t2 = t3;
  assign t1 = 1.25;
  assign t2 = 2.5;
  assign t3 = 5.0;

  N pa, pb;
  sub_inout u_io(pa, pb);
  assign pa = 1.25;
  assign pb = 2.5;

  N pia;
  real loc_r;
  sub_loc u_loc(pia, loc_r);
  assign pia = 1.25;

  real priv_r;
  sub_priv u_priv(priv_r);

  N arr1[2], arr2[2];
  alias arr1 = arr2;
  drv_r u_r0(arr1[0], 1.25);
  drv_r u_r1(arr2[0], 2.5);
  drv_r u_r2(arr1[1], 10.0);
  drv_r u_r3(arr2[1], 20.0);

  NS sarr1[2], sarr2[2];
  alias sarr1 = sarr2;
  drv_s u_s0(sarr1[0], 1.25, 3);
  drv_s u_s1(sarr2[0], 2.5, 7);
  child_arr_elt u_ce(sarr2[1]);
  drv_s u_s2(sarr1[1], 10.0, 6);

  generate
    if (1) begin : g
      N ga, gb;
      alias ga = gb;
      assign ga = 1.5;
      assign gb = 2.5;
    end
  endgenerate

  UR ua, ub;
  alias ua = ub;
  assign ua = 4.5;

  US usa, usb;
  alias usa = usb;
  assign usa = '{4.5, 9};

  N da, db;
  alias da = db;
  assign #2 da = 1.25;
  assign #4 db = 2.5;

  N ia = 1.25;
  N ib;
  alias ia = ib;
  assign ib = 2.5;

  NSA saa, sab;
  alias saa = sab;
  assign saa = '{a:'{1, 2}};
  assign sab = '{a:'{3, 4}};

  wor wa, wb;
  sub_wor u_w(wa, wb);
  assign wb = 1'b0;

  initial begin
    #1;
    $display("C1=%g,%g", a, b);
    $display("C2=%g,%0d|%g,%0d", sa.value, sa.count, sb.value, sb.count);
    $display("C3=%g,%g,%g", c1, c2, c3);
    $display("C4=%g,%g,%g", t1, t2, t3);
    $display("C5=%g,%g", pa, pb);
    $display("C6=%g,%g", pia, loc_r);
    $display("C7=%g", priv_r);
    $display("C8=%g,%g|%g,%g", arr1[0], arr2[0], arr1[1], arr2[1]);
    $display("C8S=%g,%0d|%g,%0d|%g,%0d|%g,%0d",
             sarr1[0].value, sarr1[0].count, sarr2[0].value, sarr2[0].count,
             sarr1[1].value, sarr1[1].count, sarr2[1].value, sarr2[1].count);
    $display("C8G=%g,%g", g.ga, g.gb);
    $display("C9=%g,%g|%g,%0d|%g,%0d", ua, ub, usa.value, usa.count, usb.value, usb.count);
    $display("C10=%b,%b", wa, wb);
    $display("C12=%g,%g", ia, ib);
    $display("C13=%0d,%0d|%0d,%0d", saa.a[0], saa.a[1], sab.a[0], sab.a[1]);
    #2;
    $display("C11_T3=%g,%g", da, db);
    #2;
    $display("C11_T5=%g,%g", da, db);
    $finish;
  end
endmodule
"#;
    let sim = xezim::simulate(SV, 100_000).expect("simulate nettype alias");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let expected = [
        "C1=3.75,3.75",
        "C2=3.75,10|3.75,10",
        "C3=8.75,8.75,8.75",
        "C4=8.75,8.75,8.75",
        "C5=3.75,3.75",
        "C6=3.75,3.75",
        "C7=3.75",
        "C8=3.75,3.75|30,30",
        "C8S=3.75,10|3.75,10|11.5,10|11.5,10",
        "C8G=4,4",
        "C9=4.5,4.5|4.5,9|4.5,9",
        "C10=1,1",
        "C12=3.75,3.75",
        "C13=4,6|4,6",
        "C11_T3=1.25,1.25",
        "C11_T5=3.75,3.75",
    ];
    for e in expected {
        assert!(
            msgs.iter().any(|m| m == e),
            "expected '{e}' in output, got: {msgs:#?}"
        );
    }
}

#[test]
fn issue_1302_unresolved_nettype_alias_two_drivers_error() {
    const SV_SCALAR: &str = r#"
module top;
  nettype real UR;
  UR a, b;
  alias a = b;
  assign a = 1.0;
  assign b = 2.0;
endmodule
"#;
    let err = match xezim::simulate(SV_SCALAR, 100) {
        Err(e) => e,
        Ok(_) => panic!("two drivers across scalar alias must error"),
    };
    assert!(
        err.contains("continuous drivers"),
        "unexpected error message: {err}"
    );

    const SV_STRUCT: &str = r#"
module top;
  typedef struct { real value; int count; } S;
  nettype S US;
  US a, b;
  alias a = b;
  assign a = '{1.0, 2};
  assign b = '{3.0, 4};
endmodule
"#;
    let err = match xezim::simulate(SV_STRUCT, 100) {
        Err(e) => e,
        Ok(_) => panic!("two drivers across struct alias must error"),
    };
    assert!(
        err.contains("continuous drivers"),
        "unexpected error message: {err}"
    );
}
