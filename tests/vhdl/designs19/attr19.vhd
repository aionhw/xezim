-- VHDL-2019 attributes: 'IMAGE and TO_STRING of composite values
-- (§16.2.2, §5.7), the subtype attributes 'SUBTYPE, 'ELEMENT, 'BASE,
-- 'DESIGNATED_SUBTYPE and 'INDEX as type marks, and 'VALUE of scalar types.
use std.textio.all;
entity attr19 is end entity;
architecture a of attr19 is
  type rec is record
    a : integer;
    b : boolean;
    c : character;
  end record;
  type arr is array (1 to 3) of integer;
  type nest is record
    r : rec;
    v : bit_vector(3 downto 0);
    x : arr;
  end record;
  type grid is array (natural range <>, natural range <>) of bit;
  type iptr is access integer;
  type color is (red, green, blue);
  type dist is range 0 to 1000000
    units
      mm;
      m = 1000 mm;
    end units;
  subtype small is integer range 1 to 9;
  signal sv : bit_vector(3 downto 0) := "1010";
begin
  process
    variable l : line;
    variable r : rec := (5, true, 'x');
    variable v : arr := (1, 2, 3);
    variable n : nest := ((-7, false, 'q'), "0110", (4, 5, 6));
    variable d : iptr'designated_subtype;
    variable e : integer_vector'index;
    variable q : arr'element;
    variable x : small'base;
    variable y : sv'subtype;
  begin
    write(l, rec'image(r) & " " & arr'image(v) & " " & to_string(r) & " " & to_string(v));
    writeline(output, l);
    write(l, nest'image(n) & " " & bit_vector'image(sv) & " " & to_string(n));
    writeline(output, l);
    d := 4;
    e := 2;
    q := 3;
    x := -1;
    y := "0101";
    write(l, integer'image(d) & integer'image(e) & integer'image(q) & integer'image(x)
      & " " & to_string(y) & integer'image(y'left) & integer'image(y'right));
    writeline(output, l);
    write(l, integer'image(integer'value(" -12_3 "))
      & " " & real'image(real'value(" 2.5 "))
      & " " & color'image(color'value(" GREEN ")) & " " & character'image(character'value("'x'"))
      & " " & boolean'image(boolean'value("TRUE")) & " " & dist'image(dist'value("3 m"))
      & " " & dist'image(dist'value("7 MM")));
    writeline(output, l);
    wait;
  end process;
end architecture;
