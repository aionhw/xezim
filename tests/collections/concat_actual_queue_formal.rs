//! §10.8 / §10.10: a concatenation passed to a queue formal is an unpacked
//! array concatenation, so a queue operand contributes all of its elements
//! (`{q}`, `{q, B}`), by name or by position.
#[test]
fn concat_actual_splices_queue_operands_into_queue_formal() {
    let result = xezim::simulate(
        r#"
typedef enum {A, B, C, D, E} name_t;
class lib_c;
  static function void pick(name_t inc[$] = {}, name_t exc[$] = {});
    string s;
    s = $sformatf("%0d", inc.size());
    foreach (inc[i]) s = {s, " ", inc[i].name()};
    $display("%s", s);
  endfunction
endclass
class user_c;
  name_t q[$];
  function void run();
    name_t lq[$];
    q = {C, D, E};
    lq = q;
    lib_c::pick(.inc({q}));
    lib_c::pick({lq});
    lib_c::pick({q, B});
    lib_c::pick(.inc({A, B}));
  endfunction
endclass
module t;
  user_c u;
  initial begin
    u = new();
    u.run();
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["3 C D E", "3 C D E", "4 C D E B", "2 A B"]);
}
