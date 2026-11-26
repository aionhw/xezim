//! §8.4, §7.4.5: a selected property belongs to its receiver, including when a
//! module declares a queue of the same name and dimensions.

#[test]
fn select_queue_rows_on_their_object() {
    let src = r#"
module tb;
  class K;
    int qi[$][2];
  endclass
  K a, b;
  int qi[$][2];
  initial begin
    a = new; b = new;
    a.qi.push_back('{5, 6});
    a.qi.push_back('{7, 8});
    b.qi.push_back('{9, 10});
    qi.push_back('{1, 2});
    $display("before %0d %0d %0d", a.qi[1][0], a.qi[0][1], qi[0][0]);
    a.qi[0][1] = 42;
    $display("after %0d %0d %0d", a.qi[0][1], b.qi[0][1], qi[0][1]);
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).unwrap();
    let lines: Vec<_> = sim.output.iter().map(|line| line.message.trim()).collect();
    assert_eq!(lines, ["before 7 6 1", "after 42 10 2"]);
}

#[test]
#[ignore = "open: an interpreted foreach over a class-property queue of fixed rows reads 0, also on main (§7.4.5, #1468)"]
fn whole_row_update_is_visible_inside_the_owning_method() {
    let result = xezim::simulate(
        r#"module top;
            typedef int pair_t[2];
            class K;
                pair_t rows[$];
                function int total();
                    int s = 0;
                    foreach (rows[i]) s += rows[i][1];
                    return s;
                endfunction
            endclass
            K k;
            pair_t rows[$];
            initial begin
                k = new;
                k.rows.push_back('{1, 2});
                rows.push_back('{30, 40});
                k.rows[0] = {5, 6};
                $display("T %0d %0d %0d", k.total(), k.rows[0][1], rows[0][1]);
            end
        endmodule"#,
        1000,
    )
    .unwrap();
    assert!(
        result.output.iter().any(|line| line.message == "T 6 6 40"),
        "output: {:?}",
        result
            .output
            .iter()
            .map(|line| &line.message)
            .collect::<Vec<_>>()
    );
}
