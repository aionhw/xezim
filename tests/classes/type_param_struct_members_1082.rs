//! §6.20.3, §8.25, §7.2: a variable declared through a type parameter
//! (`T loc;`) has the type the parameter is bound to. For a packed struct
//! binding its members are that struct's: a member read of a subroutine
//! local or formal declared through a class type parameter read 0 (a 1-bit
//! member printed as 32 zeros) and a member write was lost, while the whole
//! value was right; a subroutine local of a module, interface or program
//! declared through its type parameter was 32 bits wide with no members
//! (aionhw/xezim#1082, reported by Ganesh-TNL). The cases cover locals and
//! formals of every direction, collections and their formals, class-scope
//! typedefs and struct members of the parameter, fork-block locals, a static
//! property, static methods called through a handle, virtual and
//! derived-class methods, unions, a package-parameter
//! member width, a chained default, and module, body `parameter type`,
//! interface and program subroutines. Expected values are the reference
//! simulator's, on both the compiled and the interpreted method paths.

const SRC: &str = r#"package p;
  typedef struct packed { logic vld; logic [31:0] data; logic [3:0] strb; logic last; } pkt_t;
  typedef union packed { logic [37:0] all; pkt_t pkt; } upkt_t;
  parameter int W = 12;
  typedef struct packed { logic [W-1:0] d; logic v; } wpkt_t;
endpackage
import p::*;
class C #(type T = p::pkt_t);
  typedef T tt;
  typedef struct packed { T inner; logic [3:0] tag; } wrap_t;
  static T sprop;
  task automatic t_local();
    T loc;
    loc = '{vld:1'b1, data:32'h1500_0021, strb:4'hf, last:1'b1};
    $display("task local: vld=%b data=%h strb=%h last=%b whole=%h", loc.vld, loc.data, loc.strb, loc.last, loc);
  endtask
  function automatic int f_local();
    T loc;
    loc = '{vld:1'b1, data:32'hDEAD_BEEF, strb:4'h3, last:1'b0};
    return loc.data;
  endfunction
  task automatic t_formal(T item, ref T r, output T o, inout T io);
    o = '0;
    o.data = item.data + 1;
    o.vld = 1'b1;
    io.strb = 4'h7;
    $display("formals: item=%h r=%h o=%h io.strb=%h", item.data, r.data, o.data, io.strb);
  endtask
  task automatic t_writes();
    T a, b;
    a = '0;
    a.data = 32'hBBBB_CCCC;
    a.data += 32'h11;
    a.strb++;
    b = '0;
    b.data = a.data;
    b.vld = 1'b1;
    $display("writes: a=%h a.strb=%h b.data=%h b.vld=%b", a, a.strb, b.data, b.vld);
  endtask
  task automatic t_colls();
    T q[$];
    T d[];
    T aa[int];
    T fa[2];
    int s;
    q.push_back('{vld:1'b1, data:32'h0000_0001, strb:4'h1, last:1'b0});
    q.push_back('{vld:1'b0, data:32'h0000_0002, strb:4'h2, last:1'b1});
    d = new[1];
    d[0] = '{vld:1'b1, data:32'hC002_0003, strb:4'h3, last:1'b0};
    aa[5] = '{vld:1'b1, data:32'hC003_0005, strb:4'h5, last:1'b0};
    fa[1] = '{vld:1'b0, data:32'hC004_0008, strb:4'h8, last:1'b1};
    s = q.sum with (item.data);
    $display("colls: q=%h,%h d=%h aa=%h fa=%h sum=%0d", q[0].data, q[1].data, d[0].data, aa[5].data, fa[1].data, s);
    foreach (fa[i]) if (i == 1) $display("foreach: fa[%0d].strb=%h", i, fa[i].strb);
    t_coll_formals(q, d, fa);
  endtask
  task automatic t_coll_formals(T q[$], T d[], T fa[2]);
    $display("coll formals: %h %h %h", q[1].data, d[0].data, fa[1].data);
  endtask
  task automatic t_typedefs();
    tt l;
    wrap_t w;
    l = '{vld:1'b1, data:32'hC010_0011, strb:4'h1, last:1'b1};
    w = '{inner:'{vld:1'b1, data:32'hC011_0012, strb:4'h2, last:1'b0}, tag:4'hE};
    $display("typedefs: tt.data=%h wrap.inner.data=%h wrap.tag=%h", l.data, w.inner.data, w.tag);
  endtask
  task automatic t_fork();
    fork
      begin : blk
        T loc;
        loc = '{vld:1'b1, data:32'h7777_8888, strb:4'h1, last:1'b0};
        $display("fork local: %h", loc.data);
      end
    join
  endtask
  task automatic t_static_prop();
    sprop = '{vld:1'b1, data:32'hC023_0018, strb:4'h8, last:1'b0};
    $display("static prop: %h", sprop.data);
  endtask
  static function int f_static();
    T loc;
    loc = '{vld:1'b1, data:32'hC024_0019, strb:4'h9, last:1'b1};
    $display("static function: %h whole=%h bits=%0d", loc.data, loc, $bits(loc));
    return loc.data;
  endfunction
  static task t_static();
    T loc;
    loc = '{vld:1'b1, data:32'hC026_001B, strb:4'hb, last:1'b1};
    $display("static task: %h", loc.data);
  endtask
  virtual task automatic t_virt();
    T loc;
    loc = '{vld:1'b1, data:32'hC025_001A, strb:4'ha, last:1'b0};
    $display("virtual: %h", loc.data);
  endtask
endclass
class D #(type T = p::pkt_t) extends C #(T);
  task automatic t_derived();
    T loc;
    loc = '{vld:1'b1, data:32'hC028_002F, strb:4'hf, last:1'b1};
    $display("derived: %h", loc.data);
  endtask
endclass
class U #(type T = p::upkt_t);
  task automatic t();
    T u;
    u.pkt = '{vld:1'b1, data:32'hC013_002A, strb:4'h5, last:1'b0};
    $display("union: pkt.data=%h all=%h", u.pkt.data, u.all);
  endtask
endclass
class Wd #(type T = p::wpkt_t);
  task automatic t();
    T x;
    x = '{d:12'hABC, v:1'b1};
    $display("package width: d=%h v=%b whole=%h", x.d, x.v, x);
  endtask
endclass
class Ch #(type A = p::pkt_t, type B = A);
  task automatic t();
    B x;
    x = '{vld:1'b1, data:32'hC015_002B, strb:4'h5, last:1'b1};
    $display("chained: %h", x.data);
  endtask
endclass
module mt #(type MT = p::pkt_t) ();
  task automatic t();
    MT loc;
    loc = '{vld:1'b1, data:32'h5151_0001, strb:4'h1, last:1'b1};
    $display("module task: data=%h vld=%b bits=%0d", loc.data, loc.vld, $bits(loc));
  endtask
  function automatic logic [31:0] f();
    MT loc;
    loc = '{vld:1'b1, data:32'h5252_0002, strb:4'h2, last:1'b0};
    return loc.data;
  endfunction
  task t_static();
    MT loc;
    loc = '{vld:1'b1, data:32'h5353_0003, strb:4'h3, last:1'b1};
    $display("module static task: %h", loc.data);
  endtask
endmodule
module mb ();
  parameter type BT = p::pkt_t;
  task automatic t();
    BT loc;
    loc = '{vld:1'b1, data:32'h5959_0009, strb:4'h9, last:1'b1};
    $display("body parameter type: %h", loc.data);
  endtask
endmodule
interface it #(type IT = p::pkt_t) ();
  function automatic logic [31:0] f();
    IT loc;
    loc = '{vld:1'b1, data:32'h6161_0001, strb:4'h1, last:1'b0};
    return loc.data;
  endfunction
endinterface
program pg #(type PT = p::pkt_t) ();
  task automatic t();
    PT loc;
    loc = '{vld:1'b1, data:32'h7171_0001, strb:4'h1, last:1'b1};
    $display("program task: %h", loc.data);
  endtask
  initial #2 t();
endprogram
module top;
  C #(p::pkt_t) c;
  D #(p::pkt_t) dd;
  U #(p::upkt_t) uu;
  Wd #(p::wpkt_t) ww;
  Ch #(p::pkt_t) ch;
  mt um ();
  mb ub ();
  it ui ();
  pg up ();
  initial begin
    pkt_t i1, r1, o1, io1;
    c = new(); dd = new(); uu = new(); ww = new(); ch = new();
    c.t_local();
    $display("function local: %h", c.f_local());
    i1 = '{vld:1'b1, data:32'hCAFE_F00D, strb:4'h5, last:1'b1};
    r1 = '{vld:1'b1, data:32'hC019_0014, strb:4'h4, last:1'b0};
    io1 = '{vld:1'b0, data:32'hC021_0016, strb:4'h1, last:1'b0};
    c.t_formal(i1, r1, o1, io1);
    $display("after formals: o1=%h io1=%h", o1, io1);
    c.t_writes();
    c.t_colls();
    c.t_typedefs();
    c.t_fork();
    c.t_static_prop();
    void'(c.f_static());
    c.t_static();
    c.t_virt();
    dd.t_derived();
    uu.t();
    ww.t();
    ch.t();
    um.t();
    $display("module function: %h", um.f());
    um.t_static();
    ub.t();
    $display("interface function: %h", ui.f());
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    r#"task local: vld=1 data=15000021 strb=f last=1 whole=22a000043f"#,
    r#"function local: deadbeef"#,
    r#"formals: item=cafef00d r=c0190014 o=cafef00e io.strb=7"#,
    r#"after formals: o1=395fde01c0 io1=18042002ce"#,
    r#"writes: a=1777799ba2 a.strb=1 b.data=bbbbccdd b.vld=1"#,
    r#"colls: q=00000001,00000002 d=c0020003 aa=c0030005 fa=c0040008 sum=3"#,
    r#"foreach: fa[1].strb=8"#,
    r#"coll formals: 00000002 c0020003 c0040008"#,
    r#"typedefs: tt.data=c0100011 wrap.inner.data=c0110012 wrap.tag=e"#,
    r#"fork local: 77778888"#,
    r#"static prop: c0230018"#,
    r#"static function: c0240019 whole=3804800333 bits=38"#,
    r#"static task: c026001b"#,
    r#"virtual: c025001a"#,
    r#"derived: c028002f"#,
    r#"union: pkt.data=c013002a all=380260054a"#,
    r#"package width: d=abc v=1 whole=1579"#,
    r#"chained: c015002b"#,
    r#"module task: data=51510001 vld=1 bits=38"#,
    r#"module function: 52520002"#,
    r#"module static task: 53530003"#,
    r#"body parameter type: 59590009"#,
    r#"interface function: 61610001"#,
    r#"program task: 71710001"#,
];

/// Runs the design with methods compiled (`"1"`) or interpreted (`"0"`) and
/// returns its `$display` lines.
fn run(compile: &str) -> Vec<String> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "type_param_struct_members_1082_{}",
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
fn type_parameter_struct_members_compiled_and_interpreted() {
    for compile in ["1", "0"] {
        assert_eq!(run(compile), EXPECTED, "XEZIM_COMPILE_METHODS={compile}");
    }
}
