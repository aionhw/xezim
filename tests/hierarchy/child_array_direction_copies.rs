//! §7.4.2 / §7.6: whole-array assignment pairs elements left to right in
//! each side's DECLARED order. A fixed array of an inlined child module
//! (`int si [3:0];`) was registered as ascending, so assigning it to or from
//! a procedural local or a function result (`si = w;`, `si = f();`) reversed
//! the elements — while the same module run as the top, and copies between
//! two module arrays (equally misregistered), were correct.

fn output(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 1_000_000).expect("simulation failed");
    assert_eq!(result.error_count, 0, "unexpected simulation error");
    result
        .output
        .iter()
        .filter_map(|o| o.message.strip_prefix("T|").map(str::to_string))
        .collect()
}

#[test]
fn local_and_returned_arrays_copy_in_declared_order() {
    let got = output(
        r#"
module sub;
  int si [3:0];
  typedef int fat [3:0];
  function automatic fat f();
    int v [3:0];
    v[3] = 3; v[2] = 2; v[1] = 1; v[0] = 0;
    return v;
  endfunction
  initial begin
    int w [3:0];
    w[3] = 3; w[0] = 0;
    si = w; $display("T|copy %0d %0d", si[3], si[0]);
    si = f(); $display("T|ret %0d %0d", si[3], si[0]);
  end
endmodule
module top; sub dut(); endmodule
"#,
    );
    assert_eq!(got, ["copy 3 0", "ret 3 0"], "{got:?}");
}

#[test]
fn mixed_directions_and_bases_map_positionally() {
    let got = output(
        r#"
module sub;
  int md [3:0];
  int ma [0:3];
  int mb [5:2];
  logic [7:0] ml [3:0];
  typedef int fat [3:0];
  fat mt;
  function automatic fat f();
    int v [3:0];
    for (int i = 0; i < 4; i++) v[i] = 10 + i;
    return v;
  endfunction
  initial begin
    int ld [3:0];
    int la [0:3];
    int lb [5:2];
    logic [7:0] ll [3:0];
    for (int i = 0; i < 4; i++) begin md[i] = 20 + i; ma[i] = 30 + i; end
    for (int i = 2; i < 6; i++) mb[i] = 40 + i;
    ld = md;  $display("T|ld=md %0d %0d %0d %0d", ld[3], ld[2], ld[1], ld[0]);
    la = md;  $display("T|la=md %0d %0d %0d %0d", la[0], la[1], la[2], la[3]);
    ld = ma;  $display("T|ld=ma %0d %0d %0d %0d", ld[3], ld[2], ld[1], ld[0]);
    lb = md;  $display("T|lb=md %0d %0d %0d %0d", lb[5], lb[4], lb[3], lb[2]);
    md = lb;  $display("T|md=lb %0d %0d %0d %0d", md[3], md[2], md[1], md[0]);
    ma = ld;  $display("T|ma=ld %0d %0d %0d %0d", ma[0], ma[1], ma[2], ma[3]);
    mb = la;  $display("T|mb=la %0d %0d %0d %0d", mb[5], mb[4], mb[3], mb[2]);
    la = mb;  $display("T|la=mb %0d %0d %0d %0d", la[0], la[1], la[2], la[3]);
    ml[3] = 8'h33; ml[2] = 8'h22; ml[1] = 8'h11; ml[0] = 8'h00;
    ll = ml;  $display("T|ll=ml %h %h %h %h", ll[3], ll[2], ll[1], ll[0]);
    ll[3] = 8'hx3; ml = ll; $display("T|ml=ll %h %h", ml[3], ml[0]);
    md = f(); $display("T|md=f %0d %0d", md[3], md[0]);
    ma = f(); $display("T|ma=f %0d %0d", ma[0], ma[3]);
    mb = f(); $display("T|mb=f %0d %0d", mb[5], mb[2]);
    mt = f(); $display("T|mt=f %0d %0d", mt[3], mt[0]);
    la = f(); $display("T|la=f %0d %0d", la[0], la[3]);
    ma = md; $display("T|ma=md %0d %0d", ma[0], ma[3]);
    $display("T|p %p %p %p", md, ma, mb);
  end
endmodule
module top; sub dut(); endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "ld=md 23 22 21 20",
            "la=md 23 22 21 20",
            "ld=ma 30 31 32 33",
            "lb=md 23 22 21 20",
            "md=lb 23 22 21 20",
            "ma=ld 30 31 32 33",
            "mb=la 23 22 21 20",
            "la=mb 23 22 21 20",
            "ll=ml 33 22 11 00",
            "ml=ll x3 00",
            "md=f 13 10",
            "ma=f 13 10",
            "mb=f 13 10",
            "mt=f 13 10",
            "la=f 13 10",
            "ma=md 13 10",
            "p '{13, 12, 11, 10} '{13, 12, 11, 10} '{13, 12, 11, 10}",
        ],
        "{got:?}"
    );
}
