//! §25.9, §7.6, §11.4.5 (#1311): a variable of an interface is reached through a virtual
//! interface as it is through a hierarchical name. A whole unpacked variable (an array, a
//! structure, a queue, a dynamic or associative array, a member of a structure that is one)
//! through a virtual interface was resolved by the element and scalar paths only: the
//! whole-variable copy, read and comparison looked the operand up by its written name, found
//! nothing for a handle held in a class property, a static property, a package variable, an
//! array element, a subroutine formal or a function local, and did nothing. Such an operand
//! is now the bound instance's variable on either side of a blocking or nonblocking
//! assignment and of an equality operator, and as a subroutine actual.

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, expected);
}

/// Whole-array, queue and dynamic-array writes, reads and a comparison through class
/// properties (with and without a modport), a task formal and a class property used from
/// module code.
#[test]
fn whole_variable_through_class_held_and_formal_handles() {
    check(
        r#"
interface share_if;
  longint arr [8];
  typedef struct { int k; longint a [4]; } u_t;
  u_t uvar;
  int q [$];
  int darr [];
  logic [7:0] m2d [2][3];
  modport WRITER (output arr, uvar, m2d, ref q, ref darr);
  modport READER (input arr, uvar, m2d, ref q, ref darr);
endinterface
class W;
  virtual share_if.WRITER vif;
  virtual share_if.READER rif;
  virtual share_if pif;
  task wr();
    longint wrv [8]; longint rd [8]; int lq [$]; int ld [];
    foreach (wrv[i]) wrv[i] = i + 1;
    vif.arr = wrv;
    rd = rif.arr;
    $display("T|C1 wr arr0=%0d arr7=%0d  C3 rd0=%0d rd7=%0d  C4 eq=%0d", pif.arr[0], pif.arr[7], rd[0], rd[7], rif.arr == wrv);
    lq = '{4, 5, 6};
    vif.q = lq;
    $display("T|C18 q.size=%0d", pif.q.size());
    lq = '{};
    lq = rif.q;
    $display("T|C18 rd size=%0d", lq.size());
    pif.q = '{7, 8};
    $display("T|C19 q.size=%0d", pif.q.size());
    ld = new[3]; ld = '{1, 2, 3};
    vif.darr = ld;
    $display("T|C20 darr.size=%0d", pif.darr.size());
    ld = new[0];
    ld = rif.darr;
    $display("T|C20 rd size=%0d", ld.size());
  endtask
endclass
module top;
  share_if u_if ();
  virtual share_if.WRITER mh;
  W w;
  task automatic tk(virtual share_if.WRITER h);
    longint wrv [8];
    foreach (wrv[i]) wrv[i] = 20 + i;
    h.arr = wrv;
  endtask
  initial begin
    longint wrv [8];
    w = new; w.vif = u_if; w.rif = u_if; w.pif = u_if; mh = u_if;
    w.wr();
    foreach (wrv[i]) wrv[i] = 10 + i;
    mh.arr = wrv;
    $display("T|C13 arr0=%0d", u_if.arr[0]);
    tk(u_if);
    $display("T|C7 arr0=%0d", u_if.arr[0]);
    foreach (wrv[i]) wrv[i] = 30 + i;
    w.vif.arr = wrv;
    $display("T|C21 arr0=%0d", u_if.arr[0]);
  end
endmodule
"#,
        &[
            "T|C1 wr arr0=1 arr7=8  C3 rd0=1 rd7=8  C4 eq=1",
            "T|C18 q.size=3",
            "T|C18 rd size=3",
            "T|C19 q.size=2",
            "T|C20 darr.size=3",
            "T|C20 rd size=3",
            "T|C13 arr0=10",
            "T|C7 arr0=20",
            "T|C21 arr0=30",
        ],
    );
}

/// The other unpacked types (multidimensional array, structure, structure member, array of
/// structures, associative array), the nonblocking form, a pattern, a subroutine actual and
/// an array method, and handles in a static property, a package, an array element and a
/// function local.
#[test]
fn whole_variable_types_and_handle_scopes() {
    check(
        r#"
typedef struct { int k; longint a [4]; } u_t;
interface share_if;
  longint arr [8];
  u_t uvar;
  int q [$];
  int darr [];
  int aa [string];
  logic [7:0] m2d [2][3];
  u_t sav [2];
  modport WRITER (output arr, uvar, m2d, sav, ref q, ref darr, ref aa);
  modport READER (input arr, uvar, m2d, sav, ref q, ref darr, ref aa);
endinterface
package pk;
  virtual share_if.WRITER ph;
endpackage
class W;
  virtual share_if.WRITER vif;
  virtual share_if.READER rif;
  static virtual share_if.WRITER sh;
  function int sum8(longint a [8]); int s = 0; foreach (a[i]) s += a[i]; return s; endfunction
  task run();
    longint wrv [8]; longint rd [8]; logic [7:0] lm [2][3]; logic [7:0] rm [2][3];
    u_t lu, ru; u_t ls [2]; longint la [4]; int laa [string];
    foreach (wrv[i]) wrv[i] = i + 1;
    vif.arr <= wrv;
    #1 $display("T|nba arr0=%0d arr7=%0d", rif.arr[0], rif.arr[7]);
    foreach (lm[i, j]) lm[i][j] = 10 * i + j;
    vif.m2d = lm; rm = rif.m2d;
    $display("T|m2d w=%0d r=%0d", rif.m2d[1][2], rm[1][2]);
    lu.k = 7; lu.a = '{1, 2, 3, 4};
    vif.uvar = lu; ru = rif.uvar;
    $display("T|struct k=%0d a3=%0d rk=%0d ra3=%0d eq=%0d", rif.uvar.k, rif.uvar.a[3], ru.k, ru.a[3], rif.uvar == lu);
    la = '{5, 6, 7, 8};
    vif.uvar.a = la;
    la = '{0, 0, 0, 0};
    la = rif.uvar.a;
    $display("T|member a0=%0d la3=%0d", rif.uvar.a[0], la[3]);
    ls[0] = lu; ls[1] = lu; ls[1].k = 9;
    vif.sav = ls;
    $display("T|sav k=%0d,%0d", rif.sav[0].k, rif.sav[1].k);
    $display("T|call sum=%0d", sum8(rif.arr));
    vif.arr = '{default: 2};
    $display("T|pattern arr3=%0d sum=%0d", rif.arr[3], rif.arr.sum());
    laa["a"] = 1; laa["b"] = 2;
    vif.aa = laa; laa.delete(); laa = rif.aa;
    $display("T|assoc n=%0d rn=%0d", rif.aa.num(), laa.num());
    foreach (wrv[i]) wrv[i] = 40 + i;
    sh.arr = wrv;
    $display("T|static arr0=%0d", rif.arr[0]);
    foreach (wrv[i]) wrv[i] = 50 + i;
    pk::ph.arr = wrv;
    $display("T|pkg arr0=%0d", rif.arr[0]);
    if (rif.arr != wrv) $display("T|neq wrong"); else $display("T|neq ok");
  endtask
endclass
module top;
  share_if u_if ();
  virtual share_if.WRITER varr [2];
  W w;
  function automatic void fl();
    virtual share_if.WRITER lh = varr[0];
    longint wrv [8];
    foreach (wrv[i]) wrv[i] = 80 + i;
    lh.arr = wrv;
  endfunction
  initial begin
    longint wrv [8];
    w = new; w.vif = u_if; w.rif = u_if; W::sh = u_if; pk::ph = u_if; varr[0] = u_if;
    w.run();
    foreach (wrv[i]) wrv[i] = 60 + i;
    varr[0].arr = wrv;
    $display("T|elem arr0=%0d", u_if.arr[0]);
    fl();
    $display("T|fnlocal arr0=%0d", u_if.arr[0]);
  end
endmodule
"#,
        &[
            "T|nba arr0=1 arr7=8",
            "T|m2d w=12 r=12",
            "T|struct k=7 a3=4 rk=7 ra3=4 eq=1",
            "T|member a0=5 la3=8",
            "T|sav k=7,9",
            "T|call sum=36",
            "T|pattern arr3=2 sum=16",
            "T|assoc n=2 rn=2",
            "T|static arr0=40",
            "T|pkg arr0=50",
            "T|neq ok",
            "T|elem arr0=60",
            "T|fnlocal arr0=80",
        ],
    );
}
