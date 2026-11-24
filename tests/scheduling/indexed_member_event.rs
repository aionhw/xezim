//! §9.4.2. Only changes to the selected member wake its process.

fn delta(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 10_000).unwrap();
    result
        .output
        .iter()
        .map(|line| line.message.clone())
        .filter(|line| line.starts_with("DELTA"))
        .collect()
}

/// A member of a selected packed-structure element (`sa[1].a`).
#[test]
fn indexed_packed_member_event_tracks_its_own_bits() {
    let lines = delta(
        r#"
module tb;
  typedef struct packed {logic a; logic b;} word_t;
  word_t sa[2];
  int ns, before_s;
  always @(sa[1].a) ns++;
  initial begin
    sa[0] = '0; sa[1] = '0;
    #1; before_s = ns;
    sa[1].b = 1; sa[0].a = 1;
    #1; sa[1].a = 1;
    #1; sa[1].a = 1;
    #1; sa[1].a = 0;
    #1; $display("DELTA %0d", ns-before_s);
    $finish;
  end
endmodule"#,
    );
    assert_eq!(lines, ["DELTA 2"]);
}

/// A property of a selected class handle (`ca[1].v`), including a
/// replacement of the handle in the array.
#[test]
fn indexed_member_event_tracks_values_and_receiver_replacement() {
    let lines = delta(
        r#"
module tb;
  class C; int v; endclass
  C ca[2], replacement;
  int nc, before_c;
  always @(ca[1].v) nc++;
  initial begin
    ca[0] = new; ca[1] = new; replacement = new;
    #1; before_c = nc;
    ca[0].v = 1;
    #1; ca[1].v = 5;
    #1; ca[1].v = 5;
    #1; ca[1] = replacement;
    #1; replacement.v = 7;
    #1; $display("DELTA %0d", nc-before_c);
    $finish;
  end
endmodule"#,
    );
    assert_eq!(lines, ["DELTA 3"]);
}

/// A procedural wait on a property of a selected class handle wakes on each
/// change of the property.
#[test]
fn procedural_wait_on_a_selected_handle_property() {
    let lines = delta(
        r#"
module tb;
  class C; int v; endclass
  C ca[2];
  int n;
  initial begin
    ca[0] = new; ca[1] = new;
    #1; forever begin @(ca[1].v); n++; end
  end
  initial begin
    #2 ca[1].v = 5;
    #1 ca[0].v = 6;
    #1 ca[1].v = 7;
    #1 $display("DELTA %0d", n);
    $finish;
  end
endmodule"#,
    );
    assert_eq!(lines, ["DELTA 2"]);
}
