library ieee;
use ieee.std_logic_1164.all;

entity std_logic is
end entity;

architecture sim of std_logic is
begin
  process
    variable a, b : std_ulogic;
    variable v : std_ulogic_vector(3 downto 0) := "01XZ";
    variable w : std_logic_vector(0 to 3);
  begin
    a := '1'; b := 'H';
    report "and: " & std_ulogic'image(a and b);
    report "or: " & std_ulogic'image('0' or 'L');
    report "xor: " & std_ulogic'image(a xor 'X');
    report "not: " & std_ulogic'image(not 'Z');
    report "to_x01: " & std_ulogic'image(to_x01('H'));
    report "vec: " & to_string(v);
    w := std_logic_vector(v) and "1100";
    report "and vec: " & to_string(w);
    report "not vec: " & to_string(not v);
    report "hex: " & to_hstring(std_ulogic_vector'("10100101"));
    report "is_x: " & boolean'image(is_x(v));
    report "resolved: " & std_ulogic'image(resolved(std_ulogic_vector'("1Z")));
    report "resolved2: " & std_ulogic'image(resolved(std_ulogic_vector'("10")));
    report "match: " & std_ulogic'image('1' ?= 'H');
    report "to_bit: " & bit'image(to_bit('H'));
    report "bv: " & to_string(to_stdulogicvector(bit_vector'("1010")));
    wait;
  end process;
end architecture;
