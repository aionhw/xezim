//! IEEE 1800-2023 §23.9: a method local or formal shadows a class array
//! constant of the same name. `$size`, a whole-array copy and `foreach`
//! resolved the bare name to the class constant while an element read
//! used the local (#534, reported by akashlevy).

use xezim::simulate;

#[test]
fn method_local_array_shadows_the_class_constant() {
    let src = r#"
class C;
  localparam int T[2] = '{1, 2};
  function void show();
    int T[4] = '{5, 6, 7, 8};
    int x[4];
    $display("size=%0d", $size(T));
    x = T;
    $display("x3=%0d", x[3]);
    foreach (T[k]) $display("T[%0d]=%0d", k, T[k]);
  endfunction
  function void formal(int T[3]);
    int n = 0;
    foreach (T[k]) n += T[k];
    $display("fsize=%0d fsum=%0d", $size(T), n);
  endfunction
  function void own();
    $display("own=%0d", $size(T));
  endfunction
endclass
module t;
  initial begin
    C c = new;
    c.show();
    c.formal('{10, 20, 30});
    c.own();
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    for m in ["size=4", "x3=8", "T[0]=5", "T[3]=8", "fsize=3 fsum=60", "own=2"] {
        assert!(msgs.contains(&m), "missing {m}: {msgs:?}");
    }
}

/// An unpacked-array FORMAL of a class method named like a class array
/// constant shadows it too (§13.5): the bare name was lifted to the object's
/// member storage before the frame's formal was consulted.
#[test]
fn method_array_formal_shadows_the_class_constant() {
    let src = r#"
class C;
  localparam int T[2] = '{1, 2};
  function void formal(int T[3]);
    int n = 0;
    foreach (T[k]) n += T[k];
    $display("fsize=%0d fsum=%0d t2=%0d", $size(T), n, T[2]);
  endfunction
endclass
module t;
  initial begin
    C c = new;
    c.formal('{10, 20, 30});
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"fsize=3 fsum=60 t2=30"), "{msgs:?}");
}
