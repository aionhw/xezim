-- VHDL-2019 §9.1 conditional expressions whose values have different
-- lengths: the value is the selected one, with its own bounds. nvc 1.23.1
-- fails on these (an internal error), so the golden is written from the
-- LRM (condlen19.lrm).
use std.textio.all;
entity condlen19 is end entity;
architecture a of condlen19 is
  function len(v : bit_vector) return natural is
  begin
    return v'length;
  end function;
begin
  process
    variable l : line;
    variable n : integer;
  begin
    for k in -1 to 1 loop
      n := k;
      word : block
        constant w : string := "neg" when n < 0 else "zero" when n = 0 else "positive";
      begin
        write(l, w & integer'image(w'length) & " "
          & integer'image(len("01" when n < 0 else "0110")));
        writeline(output, l);
      end block word;
    end loop;
    wait;
  end process;
end architecture;
