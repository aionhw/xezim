//! §20.6.1: `$typename` of a real-valued expression is `real`. A time
//! literal is realtime (§5.8), so `$typename(1ns)` is `real` too (#324,
//! reported by Dragon-git); it, and every other expression that was not a
//! bare name, read `logic`. §6.12 makes `realtime` a synonym for `real`, and
//! the reference simulator names it so. Expected lines are the reference
//! simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .flat_map(|o| o.message.lines())
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn typename_of_real_expressions_is_real() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  typedef realtime rt_t;
  real r; realtime rt; shortreal sr; int i; rt_t v;
  function real fr(); return 1; endfunction
  initial begin
    $display("T|lit 1ns=%s 1.5=%s 1ps=%s 2.5ns=%s neg=%s", $typename(1ns), $typename(1.5), $typename(1ps), $typename(2.5ns), $typename(-1.5));
    $display("T|ops r2=%s div=%s plus=%s cond=%s", $typename(r*2), $typename(7ns/2ns), $typename(1ns + 1), $typename(1 ? 1.5 : 2.5));
    $display("T|calls fr=%s itor=%s", $typename(fr()), $typename($itor(3)));
    $display("T|vars r=%s rt=%s sr=%s i=%s v=%s kw=%s td=%s", $typename(r), $typename(rt), $typename(sr), $typename(i), $typename(v), $typename(realtime), $typename(rt_t));
    $display("T|values 7ns/2ns=%f t/2ns=%f 1ns=%f", 7ns/2ns, rt/2ns, 1ns);
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|lit 1ns=real 1.5=real 1ps=real 2.5ns=real neg=real",
            "T|ops r2=real div=real plus=real cond=real",
            "T|calls fr=real itor=real",
            "T|vars r=real rt=real sr=shortreal i=int v=real kw=real td=real",
            "T|values 7ns/2ns=3.500000 t/2ns=0.000000 1ns=1.000000",
        ]
    );
}

/// The type of an INTEGRAL expression (`$typename(7)`, `$typename(i + 1)`)
/// and the unpacked dimensions of an array variable (§20.6.1, §11.6,
/// §11.8.1): an expression takes its self-determined type — an operand's
/// own type when the result keeps its width, signedness and state — and an
/// array appends `$[l:r]` per unpacked dimension.
#[test]
fn typename_of_integral_expressions_and_arrays() {
    const SRC: &str = r#"module top;
  int ia[2]; logic [7:0] la[0:3]; real ra[3]; byte b; logic [7:0] l; int i;
  initial begin
    $display("T|ia=%s la=%s ra=%s", $typename(ia), $typename(la), $typename(ra));
    $display("T|e7=%s ip1=%s bp1=%s bb=%s lp1=%s h=%s b1=%s cat=%s", $typename(7), $typename(i+1), $typename(b+1), $typename(b+b), $typename(l+1), $typename(8'hff), $typename(1'b1), $typename({l,l}));
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|ia=int$[0:1] la=reg[7:0]$[0:3] ra=real$[0:2]",
            "T|e7=bit signed[31:0] ip1=int bp1=bit signed[31:0] bb=byte lp1=reg[31:0] h=bit[7:0] b1=bit cat=reg[15:0]",
        ]
    );
}

/// Neighbours: declared vectors and atoms (`logic` reads `reg`, no space
/// before a range, `int unsigned` reads `int`), a 2-D and a descending
/// array, and selects, comparisons, unary, shift and conditional operators.
#[test]
fn typename_declared_types_and_operators() {
    const SRC: &str = r#"module top;
  typedef logic signed [15:0] s16_t;
  int ia[2]; logic [7:0] la[0:3]; real ra[3]; byte b; logic [7:0] l; int i; logic [9:0] v10; s16_t s16; bit [3:0] b4; logic sl;
  int m2[2][3]; byte bd[3:1]; integer ig; reg [3:0] r4; shortint si; longint li; bit signed [4:0] bs; int unsigned iu;
  initial begin
    $display("T|ia=%s la=%s ra=%s", $typename(ia), $typename(la), $typename(ra));
    $display("T|e7=%s ip1=%s bp1=%s bb=%s lp1=%s h=%s b1=%s cat=%s", $typename(7), $typename(i+1), $typename(b+1), $typename(b+b), $typename(l+1), $typename(8'hff), $typename(1'b1), $typename({l,l}));
    $display("T|v10=%s s16=%s b4=%s sl=%s m2=%s bd=%s ig=%s r4=%s si=%s li=%s bs=%s iu=%s", $typename(v10), $typename(s16), $typename(b4), $typename(sl), $typename(m2), $typename(bd), $typename(ig), $typename(r4), $typename(si), $typename(li), $typename(bs), $typename(iu));
    $display("T|x1=%s x2=%s x3=%s x4=%s x5=%s x6=%s x7=%s x8=%s", $typename(i*2), $typename(ig+1), $typename(-b), $typename(b4 & b4), $typename(i == 1), $typename(l[3:0]), $typename(l[0]), $typename(ia[0]));
    $display("T|y1=%s y2=%s y3=%s y4=%s y5=%s y6=%s", $typename(si+si), $typename(li+1), $typename(iu+1), $typename(4'sd3), $typename('1), $typename(i ? b : b));
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|ia=int$[0:1] la=reg[7:0]$[0:3] ra=real$[0:2]",
            "T|e7=bit signed[31:0] ip1=int bp1=bit signed[31:0] bb=byte lp1=reg[31:0] h=bit[7:0] b1=bit cat=reg[15:0]",
            "T|v10=reg[9:0] s16=reg signed[15:0] b4=bit[3:0] sl=reg m2=int$[0:1][0:2] bd=byte$[3:1] ig=integer r4=reg[3:0] si=shortint li=longint bs=bit signed[4:0] iu=int unsigned",
            "T|x1=int x2=integer x3=byte x4=bit[3:0] x5=bit x6=reg[3:0] x7=reg x8=int",
            "T|y1=shortint y2=longint y3=int unsigned y4=bit signed[3:0] y5=bit y6=byte",
        ]
    );
}

/// Neighbours (§20.6.1, §11.4, §6.24, §7.4): queue, dynamic, associative,
/// multi-dimensional and typedef'd collections (`$[]`, `$[string]`, the
/// innermost dimension first); struct, union and enum typedefs named with
/// their package or module scope; struct and class members; `/`, `%` and
/// `**` (4-state results, `int` becoming `integer`), casts, `$signed`,
/// replication, a 4-state condition, function results and system
/// functions.
#[test]
fn typename_collections_named_types_and_operators() {
    const SRC: &str = r#"package pk;
  typedef struct { int a; } pst_t;
  typedef enum { PA, PB } pe_t;
  typedef union packed { logic [3:0] a; bit [3:0] b; } pu_t;
endpackage
class C; int x; byte y; logic [4:0] z; endclass
module top;
  typedef struct { int a; logic [3:0] b; } ms_t;
  typedef struct packed { logic [3:0] a; bit [3:0] b; } mps_t;
  typedef enum bit [1:0] { M0, M1 } me_t;
  typedef byte bq_t[$];
  byte b; logic [7:0] l; int i; bit [3:0] b4; logic signed [7:0] ls; shortint si; longint li; integer ig;
  int unsigned iu; bit sb; logic lg; time tm; string s;
  int dq[]; int qq[$]; int aa[string]; int md[2][3]; logic [1:0][3:0] pkd; bq_t bq;
  ms_t ms; mps_t mps; me_t me; pk::pst_t pst; pk::pe_t pe; pk::pu_t pu; me_t mea[3]; C c;
  function int fi(); return 1; endfunction
  function logic [5:0] fl(); return 1; endfunction
  initial begin
    c = new;
    $display("T|a dq=%s qq=%s aa=%s md=%s pkd=%s bq=%s tbq=%s", $typename(dq), $typename(qq), $typename(aa), $typename(md), $typename(pkd), $typename(bq), $typename(bq_t));
    $display("T|b ms=%s mps=%s me=%s pst=%s pe=%s pu=%s mea=%s tpe=%s", $typename(ms), $typename(mps), $typename(me), $typename(pst), $typename(pe), $typename(pu), $typename(mea), $typename(pk::pe_t));
    $display("T|c msb=%s mpsa=%s cx=%s cy=%s cz=%s mea0=%s me1=%s", $typename(ms.b), $typename(mps.a), $typename(c.x), $typename(c.y), $typename(c.z), $typename(mea[0]), $typename(me+1));
    $display("T|d div=%s mod=%s pow=%s p2i=%s bp=%s lid=%s sisi=%s iu2=%s tm2=%s", $typename(i/2), $typename(b4%3), $typename(i**2), $typename(2**i), $typename(b**2), $typename(li/2), $typename(si/si), $typename(iu/2), $typename(tm/2));
    $display("T|e lit=%s sgn=%s uns=%s ceq=%s cond=%s rep=%s cint=%s cu=%s csz=%s cbyte=%s", $typename(1+i), $typename($signed(l)), $typename($unsigned(b)), $typename(l===l), $typename(lg?b:b), $typename({2{b4}}), $typename(int'(l)), $typename(unsigned'(i)), $typename(4'(i)), $typename(byte'(i)));
    $display("T|f fi=%s fl=%s sz=%s ln=%s clg=%s tim=%s ss=%s ixl=%s", $typename(fi()), $typename(fl()), $typename(dq.size()), $typename(s.len()), $typename($clog2(5)), $typename($time), $typename({s,s}), $typename(i^ls));
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|a dq=int$[] qq=int$[$] aa=int$[string] md=int$[0:1][0:2] pkd=logic[1:0][3:0] bq=byte$[$] tbq=byte$[$]",
            "T|b ms=struct{int a;logic[3:0] b;}top.ms_t mps=struct packed{logic[3:0] a;bit[3:0] b;}top.mps_t me=enum{M0=2'd0,M1=2'd1}top.me_t pst=struct{int a;}pk::pst_t pe=enum{PA=32'sd0,PB=32'sd1}pk::pe_t pu=union packed{logic[3:0] a;bit[3:0] b;}pk::pu_t mea=enum{M0=2'd0,M1=2'd1}top.me_t$[0:2] tpe=enum{PA=32'sd0,PB=32'sd1}pk::pe_t",
            "T|c msb=reg[3:0] mpsa=logic[3:0] cx=int cy=byte cz=reg[4:0] mea0=enum{M0=2'd0,M1=2'd1}top.me_t me1=bit[31:0]",
            "T|d div=integer mod=reg[31:0] pow=integer p2i=reg signed[31:0] bp=reg signed[7:0] lid=reg signed[63:0] sisi=reg signed[15:0] iu2=integer unsigned tm2=time",
            "T|e lit=bit signed[31:0] sgn=reg signed[7:0] uns=bit[7:0] ceq=reg cond=reg signed[7:0] rep=reg[7:0] cint=int cu=bit[31:0] csz=bit signed[3:0] cbyte=byte",
            "T|f fi=int fl=logic[5:0] sz=int ln=int clg=bit signed[31:0] tim=time ss=string ixl=reg signed[31:0]",
        ]
    );
}
