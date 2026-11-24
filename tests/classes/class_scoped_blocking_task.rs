/// §8.23: a class-qualified blocking task keeps the current object.
#[test]
fn class_scoped_blocking_task() {
    let source = r#"
module top;
  class Base;
    int seen;
    virtual task work();
      #2;
      touch();
    endtask
    function void touch();
      seen = 7;
    endfunction
  endclass
  class Derived extends Base;
    virtual task work();
      $fatal(1, "wrong override");
    endtask
    task run();
      Base::work();
      $display("seen=%0d time=%0d", seen, $time);
    endtask
  endclass
  initial begin
    Derived obj = new;
    obj.run();
    $finish;
  end
endmodule
"#;
    let sim = xezim::simulate(source, 100).expect("simulation failed");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    assert!(messages.contains(&"seen=7 time=2"), "{messages:?}");
}
