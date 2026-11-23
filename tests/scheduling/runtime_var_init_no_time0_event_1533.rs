//! IEEE 1800-2023 §6.8, §6.21, §10.5: runtime variable declaration
//! assignments execute before time 0 (before edge snapshots and process
//! start), so initialized variables hold their values without generating time-0
//! events on `always` blocks waiting on them.

fn sim_lines(src: &str, top: &str, plusargs: &[String]) -> Vec<String> {
    xezim::simulate_multi(
        &[src.to_string()],
        1000,
        Some(top),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        plusargs,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.trim_end().to_string())
    .collect()
}

#[test]
fn runtime_var_decl_init_no_time0_event_with_and_without_plusargs() {
    let src = r#"
module t;
  reg   m1 = $test$plusargs("MODE");
  logic m2 = $test$plusargs("MODE");
  bit   m3 = $test$plusargs("MODE");
  integer r1 = 5 + $test$plusargs("MODE");
  int     r2 = 5 + $test$plusargs("MODE");
  time a1, a2, a3, b1, b2;
  always @(m1) a1 = $time;
  always @(m2) a2 = $time;
  always @(m3) a3 = $time;
  always @(r1) b1 = $time;
  always @(r2) b2 = $time;
  initial #1 begin
    $display("CHECK|vals m1=%b m2=%b m3=%b r1=%0d r2=%0d", m1, m2, m3, r1, r2);
    $display("CHECK|times a1=%0d a2=%0d a3=%0d b1=%0d b2=%0d", a1, a2, a3, b1, b2);
    $finish;
  end
endmodule
"#;
    let base: Vec<String> = sim_lines(src, "t", &[])
        .into_iter()
        .filter(|l| l.starts_with("CHECK|"))
        .collect();
    assert_eq!(
        base,
        vec![
            "CHECK|vals m1=0 m2=0 m3=0 r1=5 r2=5",
            "CHECK|times a1=x a2=x a3=x b1=x b2=x",
        ]
    );

    let with_mode: Vec<String> = sim_lines(src, "t", &["MODE=1".to_string()])
        .into_iter()
        .filter(|l| l.starts_with("CHECK|"))
        .collect();
    assert_eq!(
        with_mode,
        vec![
            "CHECK|vals m1=1 m2=1 m3=1 r1=6 r2=6",
            "CHECK|times a1=x a2=x a3=x b1=x b2=x",
        ]
    );
}

#[test]
fn runtime_var_init_functions_arrays_strings_subinstances_and_later_edges() {
    let src = r#"
module sub;
  logic sm = $test$plusargs("MODE");
  time sa;
  always @(sm) sa = $time;
endmodule

module t;
  function automatic int fval();
    return 7;
  endfunction
  reg m1 = $test$plusargs("MODE");
  int f1 = fval();
  int f2 = m1 + f1;
  int arr[2] = '{1, 2};
  string s = $sformatf("%m");
  time a_m1, a_m1_read, a_m1_star, a_m1_pos, a_f1, a_f2, a_arr, a_s, a_later;
  logic y_m1, y_comb;
  always @(m1) a_m1 = $time;
  always @(m1) begin a_m1_read = $time; y_m1 = m1; end
  always @(*) begin a_m1_star = $time; y_m1 = m1; end
  always @(posedge m1) a_m1_pos = $time;
  always_comb y_comb = m1;
  always @(f1) a_f1 = $time;
  always @(f2) a_f2 = $time;
  always @(arr[0]) a_arr = $time;
  always @(s) a_s = $time;
  always @(f1) a_later = $time;
  sub u();
  initial begin
    static int st_cnt = fval() + $test$plusargs("MODE");
    #1;
    $display("CHECK|VALS m1=%b f1=%0d f2=%0d arr0=%0d s=%s y_comb=%b sm=%b st=%0d",
             m1, f1, f2, arr[0], s, y_comb, u.sm, st_cnt);
    $display("CHECK|TIMES a_m1=%0d a_m1_read=%0d a_m1_star=%0d a_m1_pos=%0d a_f1=%0d a_f2=%0d a_arr=%0d a_s=%0d u.sa=%0d",
             a_m1, a_m1_read, a_m1_star, a_m1_pos, a_f1, a_f2, a_arr, a_s, u.sa);
    #1 f1 = f1 + 1;
    #1 $display("CHECK|LATER a_later=%0d y_comb=%b", a_later, y_comb);
    $finish;
  end
endmodule
"#;
    let base: Vec<String> = sim_lines(src, "t", &[])
        .into_iter()
        .filter(|l| l.starts_with("CHECK|"))
        .collect();
    assert_eq!(
        base,
        vec![
            "CHECK|VALS m1=0 f1=7 f2=7 arr0=1 s=t y_comb=0 sm=0 st=7",
            "CHECK|TIMES a_m1=x a_m1_read=x a_m1_star=x a_m1_pos=x a_f1=x a_f2=x a_arr=x a_s=x u.sa=x",
            "CHECK|LATER a_later=2 y_comb=0",
        ]
    );

    let with_mode: Vec<String> = sim_lines(src, "t", &["MODE=1".to_string()])
        .into_iter()
        .filter(|l| l.starts_with("CHECK|"))
        .collect();
    assert_eq!(
        with_mode,
        vec![
            "CHECK|VALS m1=1 f1=7 f2=8 arr0=1 s=t y_comb=1 sm=1 st=8",
            "CHECK|TIMES a_m1=x a_m1_read=x a_m1_star=x a_m1_pos=x a_f1=x a_f2=x a_arr=x a_s=x u.sa=x",
            "CHECK|LATER a_later=2 y_comb=1",
        ]
    );
}
