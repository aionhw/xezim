-- VHDL-2019 STD.REFLECTION: the value mirror of a non-null access value
-- mirrors the designated object (GET). nvc 1.23.1 returns a wrong value
-- here, so the golden is written from the LRM (reflacc19.lrm).
use std.textio.all;
use std.reflection.all;
entity reflacc19 is end entity;
architecture a of reflacc19 is
  type iptr is access integer;
begin
  process
    variable l : line;
    variable p : iptr := new integer'(9);
    variable vm : value_mirror;
  begin
    vm := p'reflect;
    write(l, boolean'image(vm.to_access.is_null) & " " & integer'image(vm.to_access.get.to_integer.value)
      & " " & vm.to_access.get.get_subtype_mirror.simple_name);
    writeline(output, l);
    wait;
  end process;
end architecture;
