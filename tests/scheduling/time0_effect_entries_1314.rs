//! §9.2.2.2, §4.9.1, §10.3.2 (#1314): an `always_comb` procedure is triggered once at time
//! zero, after every `initial` and `always` procedure has started; the order of a continuous
//! assignment against those procedures is open (§4.7). The settle that runs before the
//! processes start evaluated every combinational entry on the declaration-time state, which
//! shows when the evaluation does more than compute its outputs: an immediate assertion
//! failed on the type defaults, a class method ran on a null handle, a property read through
//! one ended the run, a counter counted an evaluation too many. Such an entry now waits for
//! the time-0 start batch and is evaluated once. A function that counts its calls in a
//! variable it also reads no longer makes the calling assignment sensitive to its own write.
//! Entries that only compute their outputs still settle before the processes start.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 1_000_000).expect("simulate");
    let all: Vec<_> = (sim.output.iter())
        .map(|o| o.message.trim().to_string())
        .collect();
    assert!(
        !all.iter().any(|l| l.contains("settle limit")),
        "settle limit reported:\n{}",
        all.join("\n")
    );
    let actual: Vec<_> = all.into_iter().filter(|s| s.starts_with("T|")).collect();
    assert_eq!(actual, expected);
}

/// §9.2.2.2: `always_comb` bodies with a self-counted evaluation, an immediate assertion,
/// a method call, a property read through a handle and a `$display`, in a module, an
/// interface and a generate loop.
#[test]
fn always_comb_with_effects_runs_once_after_the_initial_blocks() {
    check(
        r#"
class Box;
  int mv = 3;
  static int ncalls;
  function bit get(); ncalls++; return mv == 3; endfunction
endclass
module sub_comb;
  int msel; logic ma; int nrun; int first_saw;
  always_comb begin
    if (nrun == 0) first_saw = msel;
    nrun++;
    ma = (msel == 0);
  end
  initial msel = -1;
  int nfail; logic my;
  always_comb begin
    A: assert (msel == -1) else nfail++;
    my = (msel == -1);
  end
  Box h; logic [7:0] d; logic [7:0] y;
  always_comb y = h.get() + ^d;
  initial h = new;
  Box g; int gv;
  always_comb gv = g.mv;
  initial g = new;
  int k;
  always_comb $display("T|comb k=%0d", k);
  initial k = 5;
  int cnt; int seen;
  always_comb begin cnt += 1; seen = k; end
  initial #1 $display("T|comb nrun=%0d first=%0d ma=%b nfail=%0d my=%b calls=%0d gv=%0d cnt=%0d seen=%0d",
                      nrun, first_saw, ma, nfail, my, Box::ncalls, gv, cnt, seen);
endmodule
interface ifc;
  int isel; int n; logic o;
  always_comb begin n++; o = (isel == -1); end
  initial isel = -1;
endinterface
module top;
  sub_comb u();
  ifc i();
  for (genvar gi = 0; gi < 2; gi++) begin : gl
    int gsel; int n; logic o;
    always_comb begin n++; o = (gsel == gi); end
    initial gsel = gi;
  end
  initial #1 $display("T|scopes ifc n=%0d o=%b gen n=%0d,%0d o=%b%b", i.n, i.o, gl[0].n, gl[1].n, gl[0].o, gl[1].o);
endmodule
"#,
        &[
            "T|comb k=5",
            "T|comb nrun=1 first=-1 ma=0 nfail=0 my=1 calls=1 gv=3 cnt=1 seen=5",
            "T|scopes ifc n=1 o=1 gen n=1,1 o=11",
        ],
    );
}

/// §10.3.2: a continuous assignment whose function counts its calls, reaches a class method
/// behind a guard, reports a check or belongs to a package.
#[test]
fn continuous_assignment_function_with_effects_runs_once() {
    check(
        r#"
class Box;
  int mv = 3;
  static int ncalls;
  function bit get(); ncalls++; return mv == 3; endfunction
endclass
package pk;
  int psel; int pn;
  function automatic bit pfn(input bit raw);
    pn++;
    return (psel == -1) ? raw : ~raw;
  endfunction
endpackage
module sub_ca;
  int msel; logic [7:0] mdata; wire mprot; int ncalls; int first_saw;
  function automatic bit mfn(input bit raw, input int s);
    ncalls++;
    if (ncalls == 1) first_saw = msel;
    if (msel == s) return ~raw;
    return raw;
  endfunction
  assign mprot = mfn(^mdata, 0);
  initial msel = -1;
  // a class method behind a guard the initial block closes
  Box hbox; int bsel; wire bprot;
  function automatic bit bfn(input bit raw, input int s);
    if ((bsel == s) && hbox.get()) return ~raw;
    return raw;
  endfunction
  assign bprot = bfn(^mdata, 0);
  initial begin bsel = -1; hbox = new; end
  // a check inside the function
  int csel; wire cprot; int nbad;
  function automatic bit cfn(input bit raw, input int s);
    if (!(csel == s)) begin nbad++; $display("T|bad csel=%0d", csel); end
    return raw;
  endfunction
  assign cprot = cfn(^mdata, -1);
  initial csel = -1;
  wire pprot;
  assign pprot = pk::pfn(^mdata);
  initial pk::psel = -1;
  initial #1 $display("T|ca calls=%0d first=%0d mprot=%b box=%0d bprot=%b nbad=%0d cprot=%b pn=%0d",
                      ncalls, first_saw, mprot, Box::ncalls, bprot, nbad, cprot, pk::pn);
endmodule
module top;
  sub_ca u();
endmodule
"#,
        &["T|ca calls=1 first=-1 mprot=0 box=0 bprot=0 nbad=0 cprot=0 pn=1"],
    );
}

/// §10.3.2: entries without effects (constant and expression assignments, a net declaration
/// assignment, a plain `always_comb`, a function that only computes) are settled when an
/// `initial` block reads them at time zero, as before.
#[test]
fn entries_without_effects_settle_before_the_initial_blocks() {
    check(
        r#"
module top;
  wire [7:0] w1; assign w1 = 8'h5;
  logic [7:0] v = 3; wire [7:0] w2; assign w2 = v + 1;
  wire [7:0] w3 = v + 2;
  logic [7:0] c; always_comb c = v + 3;
  function automatic logic [7:0] dbl(input logic [7:0] a); return a * 2; endfunction
  wire [7:0] w4; assign w4 = dbl(v);
  logic [7:0] arr [3] = '{1, 2, 3}; logic [7:0] s, m;
  always_comb begin
    s = 0;
    for (int i = 0; i < 3; i++) s += arr[i];
  end
  integer j;
  always_comb begin
    m = '0;
    for (j = 0; j < 3; j = j + 1) m |= arr[j];
  end
  initial begin
    $display("T|t0 w1=%0d w2=%0d w3=%0d c=%0d w4=%0d", w1, w2, w3, c, w4);
    #1 $display("T|t1 w2=%0d w3=%0d c=%0d w4=%0d s=%0d m=%0d", w2, w3, c, w4, s, m);
  end
endmodule
"#,
        &[
            "T|t0 w1=5 w2=4 w3=5 c=6 w4=6",
            "T|t1 w2=4 w3=5 c=6 w4=6 s=6 m=3",
        ],
    );
}
