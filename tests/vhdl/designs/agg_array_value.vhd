-- An element association of a one-dimensional array aggregate whose value
-- is an array of the aggregate's type (IEEE 1076-2008 §9.3.3.3): it gives
-- as many elements as it has, positionally or across its range choice, in
-- packed (std_logic, bit) and unpacked (integer, string, array of vectors)
-- aggregates; a bit string has its own length; an unconstrained aggregate
-- is as long as all its elements together. An element of an array whose
-- element subtype is constrained by the array subtype ((open)(7 downto 0))
-- keeps that constraint.
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
entity agg_array_value is
end entity;
architecture a of agg_array_value is
  type int_arr is array (natural range <>) of integer;
  type slv_arr is array (natural range <>) of std_logic_vector(7 downto 0);
  type uslv_arr is array (natural range <>) of std_logic_vector;
  subtype b8_arr is uslv_arr(open)(7 downto 0);
  function desc(v : std_logic_vector) return string is
  begin
    return integer'image(v'length) & " " & to_string(v);
  end function;
  function ints(v : int_arr) return string is
    variable l : line;
  begin
    for i in v'range loop
      write(l, integer'image(v(i)) & " ");
    end loop;
    return l.all;
  end function;
begin
  process
    variable dst : std_logic_vector(7 downto 0);
    variable a4 : std_logic_vector(3 downto 0) := "1100";
    variable b4 : std_logic_vector(3 downto 0) := "0011";
    variable ib : int_arr(0 to 7) := (1, 2, 3, 4, 5, 6, 7, 8);
    variable ia : int_arr(0 to 7);
    variable s5 : string(1 to 5);
    variable init : slv_arr(0 to 7) := (others => x"5A");
    variable m : slv_arr(0 to 7);
    variable src8 : b8_arr(0 to 3) := (others => x"C3");
    variable w8 : b8_arr(0 to 3);
  begin
    dst := (x"A", x"5");
    report "bits " & to_string(dst);
    dst := (x"A", others => '0');
    report "bits others " & to_string(dst);
    dst := (7 downto 4 | 3 downto 0 => x"6");
    report "two choices " & to_string(dst);
    dst := (a4, b4);
    report "slices " & to_string(dst);
    report "unconstrained " & desc((a4, b4));
    report "unconstrained mixed " & desc((a4, '1', b4(1 downto 0)));
    ia := (ib(0 to 3), others => 0);
    report "int positional " & ints(ia);
    ia := (ib(4 to 7), 77, others => 0);
    report "int then scalar " & ints(ia);
    ia := (2 to 5 => ib(0 to 3), others => -1);
    report "int range " & ints(ia);
    s5 := ("ab", others => ' ');
    report "string [" & s5 & "]";
    s5 := ("ab", "cde");
    report "strings [" & s5 & "]";
    m := (0 to 3 => init(0 to 3), others => (others => '0'));
    report "vectors " & to_string(m(0)) & " " & to_string(m(3)) & " " & to_string(m(4));
    w8(1) := x"A5";
    report "open element " & to_string(w8(1)) & " " & integer'image(w8(1)'length);
    w8 := (0 to 1 => src8(2 to 3), others => (others => '1'));
    report "open slice " & to_string(w8(0)) & " " & to_string(w8(3));
    wait;
  end process;
end architecture;
