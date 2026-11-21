//! §9.4.2. Only changes to the selected member wake its process.
#[test]
fn indexed_member_event_tracks_values_and_receiver_replacement() {
    let result = xezim::simulate(
        r#"
module tb;
  class C; int v; endclass
  C ca[2], replacement;
  typedef struct packed {logic a; logic b;} word_t;
  word_t sa[2];
  int nc, ns, before_c, before_s;
  always @(ca[1].v) nc++;
  always @(sa[1].a) ns++;
  initial begin
    ca[0] = new; ca[1] = new; replacement = new;
    sa[0] = '0; sa[1] = '0;
    #1; before_c = nc; before_s = ns;
    ca[0].v = 1; sa[1].b = 1; sa[0].a = 1;
    #1; ca[1].v = 5; sa[1].a = 1;
    #1; ca[1].v = 5; sa[1].a = 1;
    #1; ca[1] = replacement; sa[1].a = 0;
    #1; replacement.v = 7;
    #1; $display("DELTA %0d %0d", nc-before_c, ns-before_s);
    $finish;
  end
endmodule"#,
        10_000,
    )
    .unwrap();
    assert!(
        result.output.iter().any(|line| line.message == "DELTA 3 2"),
        "{:?}",
        result
            .output
            .iter()
            .map(|line| &line.message)
            .collect::<Vec<_>>()
    );
}
