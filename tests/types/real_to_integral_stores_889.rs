//! §6.12.2: a real assigned to an integral variable is converted by rounding
//! to the nearest integer, ties away from zero, whatever the variable is. The
//! plain module variable and the signal-table element did; every store that
//! goes by NAME kept the f64 (#889, reported by Ganesh-TNL): an associative
//! array element (int, 4-bit, 8-bit — the DECLARED width must survive too, so
//! `$bits` stays 4 / 8), an assoc of assoc / of static array / static array of
//! assoc / queue of assoc, string-, enum- and negative-int-keyed assocs, a
//! class instance property (direct, inherited, through a two-level handle
//! chain), a static property (`S::si` and bare inside a method), a struct
//! member of an assoc element, a package struct member (which TRUNCATED), a
//! task or constructor formal copied into an int property, a sibling
//! instance's assoc element written hierarchically, a struct member of a class
//! property, a static assoc element, an `output` / `ref` formal, a compound
//! `+=` and an NBA into a property. Expected values are the reference
//! simulator's. (A `byte` member of an unpacked struct in a package or class
//! prints unsigned in xezim whatever the source — not a real-conversion
//! defect, left out of these checks.)

use xezim::simulate;

fn lines(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("design must run");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn expect(src: &str, max_time: u64, want: &[&str]) {
    let got = lines(src, max_time);
    let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "\n got: {got:#?}\nwant: {want:#?}");
}

#[test]
fn real_into_collection_elements_rounds_to_declared_width() {
    let src = r#"
typedef enum bit [2:0] { K_A = 3'd1, K_B = 3'd2 } key_e;
module mid;
  int m_arr [int];
endmodule
module child;
  initial begin
    #1;
    u_mid.m_arr[1] = 1000000.00 / 66;   // sibling instance's assoc (§23.8)
  end
endmodule
module tb;
  int  f_arr [int];
  int  f_stat [4];
  int  f_dyn [];
  int  qi [$];
  bit [3:0] nib [int];
  bit [7:0] byt [int];
  int daa [int][int];
  int aas [int][2];
  int saa [2][int];
  int qaa [$][int];
  int aai [int];
  int aas_s [string];
  int aae [key_e];
  mid   u_mid();
  child u_child();
  initial begin
    f_arr[0]  = 1000000.00 / 66;        // 15151.5151... -> 15152
    f_stat[2] = 1000000.00 / 66;
    f_dyn = new[1];
    f_dyn[0]  = 1000000.00 / 66;
    qi.push_back(1000000.00 / 66);
    qi[0] = 1000000.00 / 66;
    qi[1] = 2.5;
    nib[3] = 7.6;                        // 4-bit element: 8, still 4 bits
    byt[2] = 200.6;                      // 8-bit element: 201, still 8 bits
    daa[1][2] = 1000000.00 / 66;
    aas[5][1] = 1000000.00 / 66;
    saa[1][9] = 1000000.00 / 66;
    qaa.push_back('{default:0});
    qaa[0][3] = 1000000.00 / 66;
    aai[-7]   = 1000000.00 / 66;
    aas_s["k"] = 1000000.00 / 66;
    aae[K_B]  = 1000000.00 / 66;
    f_arr[0] += 0.6;                     // 15152 + 0.6 -> 15153
    #2;
    $display("T|F1 %0d %0d %0d %0d", f_arr[0], f_stat[2], f_dyn[0], $bits(f_arr[0]));
    $display("T|Q %0d %0d %0d", qi[0], qi[1], $bits(qi[1]));
    $display("T|N %0d %0d %0d %0d", nib[3], $bits(nib[3]), byt[2], $bits(byt[2]));
    $display("T|NEST %0d %0d %0d %0d", daa[1][2], aas[5][1], saa[1][9], qaa[0][3]);
    $display("T|NESTEQ %0d %0d %0d %0d", daa[1][2] == 15152, aas[5][1] == 15152,
             saa[1][9] == 15152, qaa[0][3] == 15152);
    $display("T|KEYS %0d %0d %0d", aai[-7], aas_s["k"], aae[K_B]);
    $display("T|HIER %0d", u_mid.m_arr[1]);
    $display("T|S %s", $sformatf("%0d", f_arr[0]));
    $finish;
  end
endmodule
"#;
    expect(
        src,
        100,
        &[
            "T|F1 15153 15152 15152 32",
            "T|Q 15152 3 32",
            "T|N 8 4 201 8",
            "T|NEST 15152 15152 15152 15152",
            "T|NESTEQ 1 1 1 1",
            "T|KEYS 15152 15152 15152",
            "T|HIER 15152",
            "T|S 15153",
        ],
    );
}

#[test]
fn real_into_class_properties_and_struct_members_rounds() {
    let src = r#"
package r2pkg;
  typedef struct { int m; int n; } st_t;
  st_t pst;
  int  pvar;
endpackage
class C;
  int  f;
  byte b8;
  bit [3:0] n4;
  real r;
  int  q [$];
  int  aa [int];
  static int sa [int];
  r2pkg::st_t st;
  function void nba_set(real v); f <= v; endfunction
endclass
class S;
  static int  si;
  static real sr;
  function void set_bare(); si = 1000000.00 / 66; endfunction
endclass
class Holder;
  int aa [int];
  function void write_elem(); aa[0] = 1000000.00 / 66; endfunction
endclass
class Base;
  int base_p;
  int base_q;
endclass
class Derived extends Base;
  int der_p;
endclass
class Hold;
  Derived d;
  function new(); d = new(); endfunction
endclass
class Box;
  int target;
  function void set_it(real r); target = r; endfunction
  task          get_it(real r); target = r; endtask
endclass
class CtorArg;
  int got;
  function new(real r); got = r; endfunction
endclass
typedef struct { int m; } el_t;
module tb;
  el_t q [$];
  el_t d [];
  el_t a [int];
  int mod_target;
  task automatic mod_set(real r); mod_target = r; endtask
  task automatic t_out(output int o); o = 2.6; endtask
  task automatic t_ref(ref int o); o = -2.5; endtask
  initial begin
    C c = new();
    S sh = new();
    Holder hh = new();
    Hold h = new();
    Derived dv = new();
    Box b = new();
    CtorArg ca;
    el_t t;
    int o1, o2;
    c.f = 1000000.00 / 66;              // integral property <- real
    c.r = 42;                           // real property <- integral
    c.b8 = 200.6;
    c.n4 = 7.6;
    c.q.push_back(2.5);
    c.aa[1] = 2.5;
    C::sa[1] = 2.5;
    c.st.m = 1000000.00 / 66;
    c.nba_set(2.5);
    S::si = 1000000.00 / 66;
    S::sr = 42;
    $display("T|P %0d %f %0d %0d %0d %0d %0d", c.f, c.r, c.b8, c.n4, c.q[0], c.aa[1], C::sa[1]);
    $display("T|PST %0d %0d", c.st.m, c.st.m == 15152);
    $display("T|S %0d %f", S::si, S::sr);
    sh.set_bare();
    $display("T|SBARE %0d", S::si);
    t.m = 0;
    q.push_back(t);
    d = new[1];
    d[0] = t;
    a[0] = t;
    q[0].m = 1000000.00 / 66;
    d[0].m = 1000000.00 / 66;
    a[0].m = 1000000.00 / 66;
    $display("T|ELEM %0d %0d %0d", q[0].m, d[0].m, a[0].m);
    hh.write_elem();
    hh.aa[2] = 1000000.00 / 66;
    $display("T|HOLD %0d %0d", hh.aa[0], hh.aa[2]);
    r2pkg::pvar  = 1000000.00 / 66;
    r2pkg::pst.m = 1000000.00 / 66;     // rounded 15151 down before
    r2pkg::pst.n = 2.5;                 // tie: away from zero -> 3
    $display("T|PKG %0d %0d %0d", r2pkg::pvar, r2pkg::pst.m, r2pkg::pst.n);
    dv.base_p  = 1000000.00 / 66;
    dv.base_q  = 1000000.00 / 66;
    dv.der_p   = 1000000.00 / 66;
    h.d.base_p = 1000000.00 / 66;
    $display("T|INH %0d %0d %0d %0d", dv.base_p, dv.base_q, dv.der_p, h.d.base_p);
    b.set_it(1000000.00 / 66);
    $display("T|METH %0d", b.target);
    b.target = 0;
    b.get_it(1000000.00 / 66);
    mod_set(1000000.00 / 66);
    ca = new(1000000.00 / 66);
    $display("T|FORMAL %0d %0d %0d", b.target, mod_target, ca.got);
    t_out(o1);
    t_ref(o2);
    #1;
    $display("T|OUT %0d %0d NBA %0d", o1, o2, c.f);
    $display("T|BITS %0d %0d", $bits(c.b8), $bits(c.aa[1]));
    $finish;
  end
endmodule
"#;
    expect(
        src,
        100,
        &[
            "T|P 15152 42.000000 -55 8 3 3 3",
            "T|PST 15152 1",
            "T|S 15152 42.000000",
            "T|SBARE 15152",
            "T|ELEM 15152 15152 15152",
            "T|HOLD 15152 15152",
            "T|PKG 15152 15152 3",
            "T|INH 15152 15152 15152 15152",
            "T|METH 15152",
            "T|FORMAL 15152 15152 15152",
            "T|OUT 3 -3 NBA 3",
            "T|BITS 8 32",
        ],
    );
}
