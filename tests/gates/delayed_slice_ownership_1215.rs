//! IEEE 1800-2023 §10.3.3 / §28.16: a delayed driver updates only its slice.
fn run(driver: &str) {
    let src = r#"
module top;
 timeunit 1ns; timeprecision 1ns;
 reg a=0; wire [1:0] q; wire g;
 DRIVER
 buf #4 b(g,a); assign q[1]=g;
 wire [3:0] chain;
 buf #1 c0(chain[0],a); buf #2 c1(chain[1],a);
 buf #3 c2(chain[2],a); buf #4 c3(chain[3],a);
 wire [95:0] wide;
 assign #5 wide[79:64]={16{a}};
 assign wide[95:80]=16'hcafe;
 assign wide[63:0]=64'h123456789abcdef0;
 initial begin
  #6;
  if(q!==0 || chain!==0 || wide!==96'hcafe0000123456789abcdef0) $fatal(1,"initial");
  a=1; #6;
  if(q!==3 || chain!==15 || wide!==96'hcafeffff123456789abcdef0) $fatal(1,"rise");
  a=0; #1 a=1; #6;
  if(q!==3 || wide!==96'hcafeffff123456789abcdef0) $fatal(1,"pulse");
  a=0; #6;
  if(q!==0 || chain!==0 || wide!==96'hcafe0000123456789abcdef0) $fatal(1,"fall");
  $display("PASS"); $finish;
 end
endmodule
"#
    .replace("DRIVER", driver);
    let sim = xezim::simulate(&src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "PASS"),
        "{:?}",
        sim.output
    );
}
#[test]
fn primitive_slice_preserves_other_bits() {
    run("buf #5 u(q[0],a);");
}
#[test]
fn continuous_slice_preserves_other_bits() {
    run("assign #5 q[0]=a;");
}

#[test]
fn checkpoint_preserves_delayed_slice_ownership() {
    use std::{fs, process::Command};
    let dir = std::env::temp_dir().join(format!("xezim_slice_checkpoint_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    fs::write(
        &src,
        r#"
module top;
 timeunit 1ns; timeprecision 1ns;
 reg a=0; wire [1:0] q; wire g;
 buf #5 u(q[0],a); buf #4 b(g,a); assign q[1]=g;
 initial begin #3; #3; if(q!==0) $fatal(1,"q=%b",q); $display("PASS restored slice"); $finish; end
endmodule
"#,
    )
    .unwrap();
    let ck = dir.join("state.ckpt");
    for extra in [
        vec!["--save-at".to_string(), format!("3ns,{}", ck.display())],
        vec!["--restore".to_string(), ck.display().to_string()],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "-s", "top"])
            .arg(&src)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("PASS restored slice"));
    }
    let _ = fs::remove_dir_all(dir);
}
