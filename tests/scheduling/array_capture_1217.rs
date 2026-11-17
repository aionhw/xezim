//! IEEE 1800 §10.10, §10.4.2, §9.4.5: deferred array stores capture elements.
#[test]
fn nonblocking_and_intra_assignment_arrays_capture_all_elements() {
    let source = r#"
module top;
  int a[$] = '{1,2};
  int f[3] = '{1,2,3};
  int s[$] = '{7,8,9};
  int b[$] = '{1,2};
  int q[$] = '{1,2};
  int d[] = '{1,2};
  int h[$];
  int k[$] = '{3,4};
  int m[$] = '{1,2};
  int n[$] = '{1,2};
  event e;
  initial begin
    a <= {a,6};
    if(a.size()!=2) $fatal(1,"NBA committed in active region");
    f <= {f[1],f[2],9};
    b <= s;
    d <= {d,3};
    h <= {5,{6,7}};
    k <= #1 {k[0],4};
    n <= @(e) {n,8};
    s[0] = 99;
    k[0] = 99;
    n[0] = 99;
    q = #1 {q,5};
    #2;
    $display("a=%p f=%p b=%p q=%p",a,f,b,q);
    $display("d=%p h=%p k=%p m=%p n=%p",d,h,k,m,n);
    $finish;
  end
  initial m = @(e) {m,9};
  initial begin #1; m[0] = 99; -> e; end
endmodule
"#;
    let out = xezim::simulate(source, 10).expect("simulate");
    assert!(
        out.output
            .iter()
            .any(|s| s.message == "a='{1, 2, 6} f='{2, 3, 9} b='{7, 8, 9} q='{1, 2, 5}"),
        "{:?}",
        out.output
    );
    assert!(
        out.output
            .iter()
            .any(|s| s.message == "d='{1, 2, 3} h='{5, 6, 7} k='{3, 4} m='{1, 2, 9} n='{1, 2, 8}"),
        "{:?}",
        out.output
    );
}

#[test]
fn checkpoint_preserves_pending_array_nba() {
    use std::{fs, process::Command};
    let dir =
        std::env::temp_dir().join(format!("xezim_array_nba_checkpoint_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    fs::write(
        &src,
        r#"
module top;
 timeunit 1ns; timeprecision 1ns;
 int q[$]='{1,2};
 initial begin
  q <= #5 {q,3};
  #1 q[0]=99;
  #5;
  if(q.size()!=3 || q[0]!=1 || q[1]!=2 || q[2]!=3) $fatal(1,"array NBA lost");
  $display("PASS restored array"); $finish;
 end
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
        assert!(String::from_utf8_lossy(&out.stdout).contains("PASS restored array"));
    }
    let _ = fs::remove_dir_all(dir);
}
