//! §8.26, §8.16: a class handle is assignment compatible with every
//! interface class its class, or a base class, implements, and with the ones
//! those interface classes extend, so `$cast` to such an interface handle
//! succeeds and assigns. It returned 0 because only the `extends` chain was
//! searched (#806). A cast to an interface the object does not implement,
//! and back from an interface handle to an unrelated class, still fails.
//! Expected C| lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
interface class Flyable; pure virtual function int fly(); endclass
interface class Drivable; pure virtual function int drive(); endclass
interface class Amphibious extends Drivable; pure virtual function int swim(); endclass
interface class Hover; pure virtual function int hover(); endclass
class Vehicle implements Drivable;
  virtual function int drive(); return 1; endfunction
endclass
class FlyingCar extends Vehicle implements Flyable;
  virtual function int fly(); return 2; endfunction
endclass
class Boat implements Amphibious;
  virtual function int drive(); return 3; endfunction
  virtual function int swim(); return 4; endfunction
endclass
module top;
  initial begin
    FlyingCar fc = new; Boat b = new; Vehicle v;
    Drivable d; Flyable f; Amphibious a; Hover h;
    FlyingCar fc2; Boat b2;
    $display("C|direct %0d", $cast(f, fc));
    $display("C|base %0d", $cast(d, fc));
    $display("C|extended %0d %0d", $cast(d, b), $cast(a, b));
    $display("C|unrelated %0d %0d", $cast(h, fc), $cast(a, fc));
    d = fc;
    $display("C|back %0d %0d", $cast(fc2, d), $cast(b2, d));
    $display("C|cross %0d", $cast(f, d));
    $display("C|calls %0d %0d %0d", d.drive(), f.fly(), a.swim());
  end
endmodule
"#;

#[test]
fn cast_to_an_implemented_interface_class_succeeds() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("C|"))
        .collect();
    assert_eq!(
        lines,
        [
            "C|direct 1",
            "C|base 1",
            "C|extended 1 1",
            "C|unrelated 0 0",
            "C|back 1 0",
            "C|cross 1",
            "C|calls 1 2 4",
        ]
    );
}
