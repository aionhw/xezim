-- Records with unconstrained elements (§5.3.3): record constraints in
-- subtypes and object declarations, a constant taking a function result's
-- bounds, element bounds after assignment, and a record signal.
entity record_constraints is end entity;
architecture a of record_constraints is
  type rec is record
    a : bit_vector;
    n : integer;
    s : string;
  end record;
  subtype rec4 is rec(a(3 downto 0), s(1 to 3));
  function mk(n : integer) return rec is
  begin return (a => (1 to n => '1'), n => n, s => "ab"); end function;
  signal r : rec4 := (a => "1010", n => 1, s => "xyz");
begin
  process
    variable v : rec(a(0 to 1), s(1 to 2));
    constant c : rec := mk(3);
  begin
    v := (a => "01", n => 2, s => "hi");
    report to_string(v.a) & " " & v.s & " " & integer'image(c.a'length) & " " & to_string(c.a) & " " & c.s
      & " " & integer'image(c.a'left) & " " & integer'image(v.a'right);
    report to_string(r.a) & " " & r.s & " " & integer'image(r.a'left) & " " & bit'image(r.a(3));
    r <= (a => "0001", n => 2, s => "abc"); wait for 1 ns;
    report to_string(r.a) & " " & r.s & " " & integer'image(r.n) & " " & bit'image(r.a(0)) & " " & integer'image(r.a'left);
    wait;
  end process;
end architecture;
