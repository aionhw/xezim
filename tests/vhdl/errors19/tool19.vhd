use std.textio.all;
use std.env.all;
entity tool19 is end entity;
architecture a of tool19 is
begin
  process
    variable l : line;
  begin
    write(l, "[" & tool_type & "|" & tool_vendor & "|" & tool_name & "|" & tool_edition & "|"
      & tool_version & "|" & vhdl_version & "]");
    writeline(output, l);
    wait;
  end process;
end architecture;
