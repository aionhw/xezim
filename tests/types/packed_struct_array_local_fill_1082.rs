//! §5.7.1, §10.6, §6.11.1: an element of an unpacked array local whose
//! element type is a packed struct or union is integral, so a store into it
//! takes the element's width: an unbased unsized fill (`'0`, `'1`, `'x`,
//! `'z`) fills every bit, and a narrower or wider value is extended or
//! truncated. The element kept the value's own width (`a[0] = '0` stored one
//! bit), so a later member write read x (aionhw/xezim#1082, check C32, reported
//! by Ganesh-TNL). The cases cover locals of initial and always_comb blocks,
//! tasks, functions and class methods (also through a class type parameter),
//! 2-state, signed and union elements, a 2-D array and the forms that were
//! right already (a packed-array element, a struct member array, a
//! nonblocking store). Expected values are the reference simulator's, on both
//! the compiled and the interpreted method paths.

const SRC: &str = r#"package p;
  typedef struct packed { logic vld; logic [31:0] data; logic [3:0] strb; logic last; } pkt_t;
  typedef struct packed { bit [3:0] hi; bit [3:0] lo; } b8_t;
  typedef struct packed signed { logic [3:0] hi; logic [3:0] lo; } s8_t;
  typedef union packed { logic [11:0] all; struct packed { logic [3:0] a; logic [7:0] b; } f; } u12_t;
  typedef logic [1:0][5:0] pa_t;
endpackage
import p::*;
class C #(type T = p::pkt_t);
  task automatic t_few();
    T a[2];
    foreach (a[i]) begin
      a[i] = '0;
      a[i].data = 32'hC032_001D + i;
      a[i].vld  = 1'b1;
    end
    $display("class T foreach: %h %h", a[0].data, a[1].data);
  endtask
  function automatic void f_conc();
    pkt_t a[2];
    a[0] = '1;
    a[0].strb = 4'h0;
    $display("class pkt: %h", a[0]);
  endfunction
endclass
module top;
  struct { pkt_t q[2]; int k; } us;
  logic a_trig;
  task automatic t_task();
    pkt_t a[2];
    a[1] = 'z;
    a[1].data = 32'h0;
    $display("task z: %h", a[1]);
  endtask
  function automatic logic [37:0] f_func();
    pkt_t a[3];
    a[2] = 'x;
    a[2].last = 1'b0;
    return a[2];
  endfunction
  initial begin
    pkt_t a[2];
    pkt_t m2[2][3];
    b8_t b[2];
    s8_t s[2];
    u12_t u[2];
    pa_t pa[2];
    C #(p::pkt_t) c;
    a[0] = '0; a[0].data = 32'h1234_5678;
    $display("init 0: %h bits=%0d", a[0], $bits(a[0]));
    a[1] = '1; $display("init 1: %h", a[1]);
    a[1] = 1'b1; $display("init 1b: %h", a[1]);
    m2[1][2] = '0; m2[1][2].strb = 4'h5; $display("2-D: %h", m2[1][2]);
    b[0] = 'x; $display("2-state x: %h", b[0]);
    s[0] = '1; $display("signed: %0d %h", s[0], s[0]);
    u[1] = '0; u[1].f.a = 4'h9; $display("union: %h", u[1]);
    pa[0] = '1; pa[0][1] = 6'h0; $display("packed array elem: %h", pa[0]);
    us.q[1] = '0; us.q[1].data = 32'hABCD_0001; $display("struct member: %h", us.q[1]);
    a[0] <= '0;
    #1 a[0].vld = 1'b1; $display("nba then member: %h", a[0]);
    t_task();
    $display("func x: %h", f_func());
    c = new(); c.t_few(); c.f_conc();
  end
  always_comb begin
    pkt_t ac[2];
    ac[0] = '0;
    ac[0].strb = 4'hc;
    a_trig = ac[0].strb[2];
  end
  initial #2 $display("always_comb local: %b", a_trig);
endmodule
"#;

const EXPECTED: &[&str] = &[
    r#"init 0: 02468acf00 bits=38"#,
    r#"init 1: 3fffffffff"#,
    r#"init 1b: 0000000001"#,
    r#"2-D: 000000000a"#,
    r#"2-state x: 00"#,
    r#"signed: -1 ff"#,
    r#"union: 900"#,
    r#"packed array elem: 03f"#,
    r#"struct member: 1579a00020"#,
    r#"nba then member: 2000000000"#,
    r#"task z: Z0000000Zz"#,
    r#"func x: xxxxxxxxxX"#,
    r#"class T foreach: c032001d c032001e"#,
    r#"class pkt: 3fffffffe1"#,
    r#"always_comb local: 1"#,
];

/// Runs the design with methods compiled (`"1"`) or interpreted (`"0"`) and
/// returns its `$display` lines.
fn run(compile: &str) -> Vec<String> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "packed_struct_array_local_fill_1082_{}",
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, SRC).expect("write design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top"])
        .arg(&path)
        .env("XEZIM_COMPILE_METHODS", compile)
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "simulation failed:\n{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| {
            !(l.is_empty()
                || l.starts_with('[')
                || l.starts_with("=== ")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished"))
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn packed_element_stores_take_the_element_width() {
    for compile in ["1", "0"] {
        assert_eq!(run(compile), EXPECTED, "XEZIM_COMPILE_METHODS={compile}");
    }
}
