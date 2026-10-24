-- Arrays (§5.3.2): constrained and unconstrained, descending ranges,
-- aggregates (positional, named, others), slices, concatenation, array
-- equality and ordering, 'range loops, bit_vector logic.
entity arrays is end entity;
architecture a of arrays is
  type int_arr is array (natural range <>) of integer;
  function sum(v : int_arr) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;
begin
  process
    variable a : int_arr(0 to 4) := (10, 20, 30, 40, 50);
    variable b : int_arr(3 downto 0) := (others => 7);
    variable bv : bit_vector(7 downto 0) := x"5A";
    variable s : string(1 to 5) := "hello";
  begin
    report "sum a=" & integer'image(sum(a)) & " sum b=" & integer'image(sum(b));
    a(2) := 99;
    b := (3 => 1, 0 => 4, others => 0);
    report "a(2)=" & integer'image(a(2)) & " b(3)=" & integer'image(b(3)) & " b(0)=" & integer'image(b(0));
    report "slice sum=" & integer'image(sum(a(1 to 3)));
    report "s=" & s & " s(2 to 4)=" & s(2 to 4) & " length=" & integer'image(s'length);
    report "cat: " & (s & " world") & " " & ('[' & s(1) & ']');
    bv := bv and x"0F";
    report "bv and: " & bit'image(bv(3)) & bit'image(bv(2)) & bit'image(bv(1)) & bit'image(bv(0));
    bv := not bv;
    report "bv not: " & bit'image(bv(7)) & bit'image(bv(0));
    report "eq: " & boolean'image(a = a) & " " & boolean'image(s = "hello") & " lt: " & boolean'image(string'("abc") < "abd");
    report "attrs: " & integer'image(b'left) & " " & integer'image(b'right) & " " & integer'image(b'high) & " " & boolean'image(b'ascending);
    wait;
  end process;
end architecture;
