//! Delayed nonblocking writes from one process commit in execution order
//! when scheduled for the same time (IEEE 1800-2023 §4.6, §4.9.4).

#[test]
fn conditional_load_overrides_default_after_the_same_delay() {
    let sim = xezim::simulate(
        r#"
        module top;
            reg clk = 0, go = 0, load = 0, busy = 0, ready = 0;
            always @(posedge clk) begin
                busy  <= #1 go;
                load  <= #1 1'b0;
                ready <= #1 1'b1;
                if (go) load <= #1 1'b1;
            end
            initial begin
                #2 go = 1;
                #3 clk = 1;
                #2 $display("load=%b busy=%b ready=%b", load, busy, ready);
                $finish;
            end
        endmodule
        "#,
        1000,
    )
    .unwrap();
    assert!(
        sim.output
            .iter()
            .any(|line| line.message == "load=1 busy=1 ready=1")
    );
}

#[test]
fn tied_updates_keep_execution_order_across_earlier_deadlines() {
    let sim = xezim::simulate(
        r#"
        module top;
            reg clk = 0, enabled = 1, other = 0, load = 0, ready = 0;
            reg later = 0;
            always @(posedge clk) begin
                other <= #1 enabled;
                later <= #3 1'b0;
                load  <= #1 1'b0;
                ready <= #1 1'b1;
                if (enabled) begin
                    load  <= #1 1'b1;
                    later <= #3 1'b1;
                end
            end
            initial begin
                #2 clk = 1;
                #2 $display("early %b%b%b%b", other, load, ready, later);
                #2 $display("late %b%b%b%b", other, load, ready, later);
                $finish;
            end
        endmodule
        "#,
        1000,
    )
    .unwrap();
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert!(lines.contains(&"early 1110"), "{lines:?}");
    assert!(lines.contains(&"late 1111"), "{lines:?}");
}
