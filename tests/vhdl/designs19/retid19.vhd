-- VHDL-2019 §4.2.1 function return identifiers: `return r of bit_vector`
-- names the subtype the call's context expects (here the targets of the
-- assignments), so the function makes a result of the target's length.
use std.textio.all;
entity retid19 is end entity;
architecture a of retid19 is
  function ones return r of bit_vector is
    variable v : r;
  begin
    v := (others => '1');
    return v;
  end function;
begin
  process
    variable l : line;
    variable a : bit_vector(3 downto 0);
    variable b : bit_vector(0 to 6);
  begin
    a := ones;
    b := ones;
    write(l, to_string(a) & " " & to_string(b) & " " & integer'image(a'length) & integer'image(b'length));
    writeline(output, l);
    wait;
  end process;
end architecture;
