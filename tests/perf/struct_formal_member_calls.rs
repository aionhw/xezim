//! §7.4.1, §13.4, §6.20: an `always_ff` whose right-hand sides call automatic
//! functions that read an element of a packed-array member of a packed-struct
//! formal (`c.gain_shift[ci]`), or that reach a helper reading the module's
//! `localparam`, runs as bytecode: every call is inlined. Each of these calls
//! used to send the whole statement to the interpreter (#1245, 50x the
//! instructions of the compiled block). The guard is the profile's fallback
//! count; the digest is the reference simulator's.

fn run_profiled(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_struct_formal_calls_{}_{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace(':', "_")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
            "--profile",
        ])
        .output()
        .expect("run profiled design");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "profiled design failed:\n{text}");
    text
}

#[test]
fn struct_formal_element_calls_compile() {
    let text = run_profiled(
        r#"
typedef struct packed {
  logic [3:0][15:0] in_off;
  logic [3:0][11:0] gain_base;
  logic [3:0][11:0] gain_hi_val;
  logic [3:0][4:0]  gain_shift;
} tune_t;

module lanes (input logic clk, input logic rst, input tune_t cfg, input logic [31:0] seed,
              output logic [63:0] digest);
  localparam int PIX_W = 12;
  function automatic logic [15:0] div_small(input logic [15:0] a, input logic [2:0] d);
    case (d)
      3'd1: return a;
      3'd2: return a >> 1;
      3'd4: return a >> 2;
      default: return '0;
    endcase
  endfunction
  function automatic logic [PIX_W-1:0] clip(input logic [39:0] v);
    if (v[39]) return '0;
    if ((v >> PIX_W) != '0) return {PIX_W{1'b1}};
    return v[PIX_W-1:0];
  endfunction
  function automatic logic [PIX_W-1:0] code_of(input logic [15:0] s, input logic [2:0] c,
                                               input logic [15:0] off);
    logic [15:0] avg;
    avg = div_small(s, c);
    return clip(40'(avg) + 40'(off));
  endfunction
  function automatic logic signed [11:0] gain_of(input logic [1:0] ci, input logic g,
                                                 input logic signed [31:0] r, input tune_t c);
    logic [4:0] sh;
    logic signed [31:0] m;
    sh = c.gain_shift[ci];
    if (g) m = 32'(signed'({1'b0, c.gain_hi_val[ci]})) + (r >>> sh);
    else m = 32'(signed'({1'b0, c.gain_base[ci]}));
    return 12'(m);
  endfunction
  function automatic logic [PIX_W-1:0] fixup(input logic [1:0] ci, input logic signed [32:0] pr,
                                             input tune_t c);
    logic signed [32:0] t;
    t = (pr + 33'(signed'({1'b0, c.in_off[ci]}))) >>> 6;
    if (t < 0) return '0;
    if (t > 33'sd4095) return {PIX_W{1'b1}};
    return PIX_W'(t);
  endfunction
  logic [PIX_W-1:0] code [4];
  logic signed [32:0] prod [4];
  logic [PIX_W-1:0] corr [4];
  always_ff @(posedge clk) begin
    if (rst) begin
      for (int l = 0; l < 4; l++) begin code[l] <= '0; prod[l] <= '0; corr[l] <= '0; end
    end else begin
      for (int l = 0; l < 4; l++) begin
        code[l] <= code_of(seed[15:0] ^ 16'(l * 977), 3'(seed[17:16] + l), cfg.in_off[3 - l]);
        prod[l] <= 33'(gain_of(2'(l + seed[1:0]), seed[l], 32'(code[l]) - 32'sd2048, cfg))
                   * 33'($signed({1'b0, code[l]}));
        corr[l] <= fixup(2'(seed[3:2] ^ l), prod[l], cfg);
      end
    end
  end
  always_ff @(posedge clk)
    if (rst) digest <= '0;
    else digest <= (digest * 64'd1000003) ^ {corr[3], corr[2], corr[1], corr[0]};
endmodule

module top;
  logic clk = 0, rst = 1;
  tune_t cfg;
  logic [31:0] seed = 32'hDEADBEEF;
  logic [63:0] digest;
  lanes u (.clk(clk), .rst(rst), .cfg(cfg), .seed(seed), .digest(digest));
  always #5 clk = ~clk;
  initial begin
    for (int l = 0; l < 4; l++) begin
      cfg.in_off[l] = 16'(l * 7919 + 12345);
      cfg.gain_base[l] = 12'(l * 613 + 100);
      cfg.gain_hi_val[l] = 12'(l * 331 + 3000);
      cfg.gain_shift[l] = 5'(l * 3 + 4);
    end
    @(negedge clk) rst = 0;
    repeat (200) begin
      @(negedge clk) seed = {seed[30:0], seed[31] ^ seed[21] ^ seed[1] ^ seed[0]};
    end
    $display("digest=%h", digest);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("digest=e14a2436faf671b7"), "{text}");
    let fallbacks = text
        .lines()
        .find(|l| l.starts_with("[PROF] edge_detect="))
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|t| t.strip_prefix("fallbacks="))
        })
        .unwrap_or_else(|| panic!("missing fallback count:\n{text}"));
    assert_eq!(fallbacks, "0", "{text}");
}

/// §7.2, §13.5: each member of a whole struct result keeps its real value.
#[test]
fn whole_struct_constructor_compiles() {
    let text = run_profiled(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real v; real i; real r; } rec;
  function automatic rec through(input rec a, input rec b);
    rec t;
    t.v = (a.v + b.v) * 0.5;
    t.i = a.i - b.i;
    t.r = a.r + b.r;
    return t;
  endfunction
  rec p, n, q;
  int k;
  always_comb q = through(p, n);
  initial begin
    for (k = 0; k < 16; k++) begin
      p.v = k; p.i = 1.0; p.r = 2.0;
      n.v = 1.0; n.i = 0.5; n.r = 3.0;
      #1;
    end
    $display("q.v=%g q.i=%g q.r=%g", q.v, q.i, q.r);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("q.v=8 q.i=0.5 q.r=5"), "{text}");
    let profile = text
        .lines()
        .find(|l| l.starts_with("[PROF] edge_detect="))
        .expect("profile");
    assert!(
        profile
            .split_whitespace()
            .any(|field| field == "fallbacks=0"),
        "{text}"
    );
}
