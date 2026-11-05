-- SHIFT_LEFT and SHIFT_RIGHT of numeric_std and numeric_bit (IEEE 1076-2008
-- §16.8.5): UNSIGNED and SIGNED, counts from 0 to past the length, and
-- metavalues, which the IEEE body moves like any element.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity numeric_shift is end entity;
architecture a of numeric_shift is
  function img(x : unsigned) return string is
  begin
    return to_string(std_logic_vector(x));
  end function;
  function img(x : signed) return string is
  begin
    return to_string(std_logic_vector(x));
  end function;
begin
  process
    variable u : unsigned(7 downto 0) := "10110011";
    variable s : signed(7 downto 0) := "10110011";
    variable p : signed(7 downto 0) := "01110010";
    variable m : unsigned(5 downto 0) := "1UX0Z1";
    variable q : unsigned(0 to 4) := "11001";
    variable bu : ieee.numeric_bit.unsigned(5 downto 0) := "110101";
    variable bs : ieee.numeric_bit.signed(5 downto 0) := "110101";
    variable n : natural;
  begin
    for c in 0 to 10 loop
      n := c;
      report integer'image(c) & " " & img(shift_left(u, n)) & " " & img(shift_right(u, n)) &
        " " & img(shift_left(s, n)) & " " & img(shift_right(s, n)) & " " & img(shift_right(p, n)) &
        " " & img(shift_left(m, n)) & " " & img(shift_right(m, n)) & " " & img(shift_left(q, n)) &
        " " & to_string(bit_vector(ieee.numeric_bit.shift_left(bu, n))) & " " & to_string(bit_vector(ieee.numeric_bit.shift_right(bu, n))) &
        " " & to_string(bit_vector(ieee.numeric_bit.shift_right(bs, n)));
    end loop;
    wait;
  end process;
end architecture;
