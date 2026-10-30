-- VHDL-2019 §16.2.2 'VALUE of composite types: the text of an aggregate of
-- the elements' values (a string literal for an array of character
-- literals). nvc 1.23.1 fails on these, so the golden is written from the
-- LRM (value19.lrm).
use std.textio.all;
entity value19 is end entity;
architecture a of value19 is
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
begin
  process
    variable l : line;
    variable r : rec;
    variable v : arr;
    variable n : nest;
    variable b : bit_vector(3 downto 0);
  begin
    r := rec'value(" (5, true, 'x') ");
    v := arr'value("(1,2,3)");
    b := bit_vector'value("""0110""");
    n := nest'value("((-7,false,'q'),""1001"",(4, 5, 6))");
    write(l, rec'image(r) & " " & arr'image(v) & " " & bit_vector'image(b) & " " & nest'image(n));
    writeline(output, l);
    v := arr'value("(1,2)");
    wait;
  end process;
end architecture;
