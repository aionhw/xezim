//! IEEE 1800-2023 §10.10. A scalar evaluation before the collection copy
//! would run each call twice, including destructive queue methods.
#[test]
fn concat_operands_run_once_before_the_destination_changes() {
    let sim = xezim::simulate(
        r#"module top;
            int src[$], dst[$], calls;
            function automatic int next_value();
                calls++;
                return calls;
            endfunction
            initial begin
                dst = {next_value(), next_value()};
                $display("T calls=%0d size=%0d sum=%0d", calls, dst.size(), dst.sum());
                src = '{1, 2, 3, 4};
                dst = {src.pop_front(), src.pop_back()};
                $display("T pops=%0d,%0d left=%0d", dst[0], dst[1], src.size());
                dst = {dst, next_value()};
                $display("T self=%0d,%0d,%0d calls=%0d", dst[0], dst[1], dst[2], calls);
            end
        endmodule"#,
        1000,
    )
    .unwrap();
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T "))
        .collect();
    assert_eq!(
        lines,
        [
            "T calls=2 size=2 sum=3",
            "T pops=1,4 left=2",
            "T self=1,4,3 calls=3",
        ]
    );
}
