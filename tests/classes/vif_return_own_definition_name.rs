//! §23.8, §25.9: inside a function of an interface instance,
//! `parent_if.children[0]` names, through the interface's DEFINITION name,
//! the instance the function runs in. Returned or assigned to a virtual
//! interface it bound nothing (the caller read x / null), while the bare
//! `children[0]` bound correctly. Expected values are the reference
//! simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn vif_bound_through_own_definition_name() {
    const SRC: &str = r#"interface child_if;
  int id;
endinterface
interface parent_if;
  child_if kid();
  child_if kids[2]();
  initial begin kid.id = 4; kids[1].id = 9; end
  function automatic virtual child_if one(); return parent_if.kid; endfunction
  function automatic virtual child_if elem(); return parent_if.kids[1]; endfunction
  function automatic virtual child_if via_local();
    virtual child_if t;
    t = parent_if.kid;
    return t;
  endfunction
endinterface
module top;
  parent_if parent();
  virtual parent_if p;
  virtual child_if c;
  initial begin
    #1;
    p = parent;
    c = p.one();
    $display("T|one null=%0d id=%0d", c == null, c.id);
    c = p.elem();
    $display("T|elem null=%0d id=%0d", c == null, c.id);
    c = parent.via_local();
    $display("T|local null=%0d id=%0d", c == null, c.id);
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|one null=0 id=4",
            "T|elem null=0 id=9",
            "T|local null=0 id=4",
        ]
    );
}

/// A value access through the same self-qualified path must reach the
/// instance, including selected children and packed slices. Interface methods
/// commonly configure per-port mode bits this way, called through a virtual
/// interface held by a class.
#[test]
fn self_qualified_interface_method_updates_selected_child() {
    const SRC: &str = r#"
interface Port;
  bit shared_clock_mode = 1;
  bit [7:0] data = 8'ha0;
endinterface
interface Bus;
  Port port_if[2]();
  function void set_mode(bit mode, int idx);
    case (idx)
      0: Bus.port_if[0].shared_clock_mode = mode;
      1: Bus.port_if[1].shared_clock_mode = mode;
    endcase
    Bus.port_if[0].data[3:0] = idx + 2;
  endfunction
  function bit read_mode(); return Bus.port_if[0].shared_clock_mode; endfunction
endinterface
class Config;
  virtual Bus vif;
  bit mode = 0;
  function void set_if(virtual Bus v);
    this.vif = v;
    this.vif.set_mode(this.mode,0);
  endfunction
endclass
module top;
  Bus b0(), b1();
  Config c0,c1;
  initial begin
    c0=new; c1=new;
    c1.mode=1;
    c0.set_if(b0); c1.set_if(b1);
    #1;
    $display("T|first mode=%0d,%0d data=%h,%h", b0.read_mode(),b1.read_mode(),b0.port_if[0].data,b1.port_if[0].data);
    b0.set_mode(0,1);
    b1.set_mode(0,0);
    #1;
    $display("T|second mode=%0d,%0d child1=%0d,%0d data=%h,%h", b0.read_mode(),b1.read_mode(),b0.port_if[1].shared_clock_mode,b1.port_if[1].shared_clock_mode,b0.port_if[0].data,b1.port_if[0].data);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|first mode=0,1 data=a2,a2",
            "T|second mode=0,0 child1=0,1 data=a3,a2",
        ]
    );
}

/// A bare name equal to the enclosing definition's name is an ordinary
/// object, not the instance: `int sub;` inside `module sub` passed as a
/// recursive task's output actual keeps its value.
#[test]
fn bare_definition_name_local_is_not_the_instance() {
    let src = r#"
module sub (input logic clk);
  task automatic fact(input int n, output int f);
    int sub;
    if (n <= 1) f = 1;
    else begin
      fact(n - 1, sub);
      f = n * sub;
    end
  endtask
  always @(posedge clk) begin
    int f;
    fact(5, f);
    $display("T|f=%0d", f);
  end
endmodule
module top;
  logic clk = 0;
  sub u (.clk(clk));
  initial begin #5 clk = 1; #5 $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|f=120"]);
}
