//! IEEE 1800-2023 §7.4.1 / §11.5.1: a store to an element, part-select or
//! bit of a packed vector wider than 64 bits edits the stored vector; it does
//! not read the whole vector out and store it back. The
//! `select_store_copies` profile counter counts the stores that did (each
//! cost several copies and scans of the vector: 1.9 ms per 32-element loop
//! over a 1-Mibit buffer in the #1210 test case). It stays 0 for module
//! variables, class properties and function locals. Expected `T|` lines
//! were recorded from the reference simulator.
use std::process::Command;

#[test]
fn wide_vector_select_stores_copy_no_vector() {
    let dir = std::env::temp_dir().join(format!("xezim_selcopies_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(
        &path,
        r#"// §7.4.1/§11.5.1: element and part-select stores into wide packed vectors.
class Buf;
  bit [1023:0][63:0] mem;
  logic [65535:0] flat;
  function void fill(int n);
    flat = '0;
    for (int i = 0; i < n; i++) begin
      mem[(i * 37) % 1024] = 64'(i) * 64'h9E3779B97F4A7C15;
      flat[((i * 53) % 8192) * 8 +: 8] = 8'(i);
      flat[(i * 11) % 65536] = i[0] ^ i[3];
    end
  endfunction
  function logic [63:0] sum();
    logic [63:0] s = 0;
    for (int i = 0; i < 1024; i += 16) s = {s[62:0], s[63]} ^ mem[i] ^ flat[i * 64 +: 64];
    return s;
  endfunction
endclass
module top;
  bit [1023:0][63:0] frame_buf;
  logic [1023:0][63:0] frame4;
  logic [63:0] beat_q[$];
  Buf b;
  function automatic logic [63:0] local_fill(int n);
    bit [1023:0][63:0] lbuf;
    logic [65535:0] lflat;
    logic [63:0] s = 0;
    lflat = '0;
    for (int i = 0; i < n; i++) begin
      beat_q.push_back(64'(i));
      lbuf[(i * 41) % 1024] = beat_q.pop_front() * 64'hD1B54A32D192ED03;
      lflat[((i * 59) % 8192) * 8 +: 8] = 8'(i);
      lflat[(i * 13) % 65536] = i[1] ^ i[2];
      lflat[40 : 33] = 8'(i);
    end
    for (int i = 0; i < 1024; i += 16) s = {s[62:0], s[63]} ^ lbuf[i] ^ lflat[i * 64 +: 64];
    return s;
  endfunction
  logic [63:0] s;
  initial begin
    for (int k = 0; k < 40; k++) begin
      for (int i = 0; i < 32; i++) beat_q.push_back({16'(k), 32'(i), 16'hBEEF});
      for (int i = 0; i < 32; i++) begin
        frame_buf[i + k] = beat_q.pop_front();
        frame4[1023 - i - k] = beat_q.size() == 5 ? 64'hx : 64'(i * k);
      end
    end
    s = 0;
    for (int i = 0; i < 1024; i += 8) s = {s[62:0], s[63]} ^ frame_buf[i];
    $display("T| mod %h %h %h %h", s, frame_buf[39], frame4[1023 - 31 - 5], frame4[1023 - 26 - 3]);
    b = new;
    b.fill(1500);
    $display("T| cls %h %h %h", b.sum(), b.mem[37], b.flat[15:0]);
    $display("T| local %h", local_fill(1500));
  end
endmodule
"#,
    )
    .expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(
        lines,
        [
            "T| mod d2806680000061ef 002700000000beef 0000000000000000 0000000000000000",
            "T| cls cc3cb7663fa7c319 7c1e5fb6a93ad015 0800",
            "T| local a42c383f1a9b5a09",
        ]
    );
    let copies: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] select_store_copies="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing select_store_copies:\n{text}"));
    assert_eq!(copies, 0, "select stores that copied the whole vector");
    let moves: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] queue_elem_moves="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing queue_elem_moves:\n{text}"));
    assert_eq!(moves, 0, "queue elements moved one by one");
}
