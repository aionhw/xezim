library ieee;
use ieee.std_logic_1164.all;
entity edgetest is end;
architecture a of edgetest is
    signal clk : std_logic := 'U';
    signal en : std_logic := '0';
    signal rises, falls : natural := 0;
begin
    process(clk) begin
        if rising_edge(clk) then
            if en = '1' then rises <= rises + 1; end if;
        end if;
    end process;
    process(clk) begin
        if falling_edge(clk) then
            if en = '1' then falls <= falls + 1; end if;
        end if;
    end process;
    process
        variable nr, nf : natural := 0;
    begin
        for e in 0 to 1 loop
            if e = 1 then en <= '1'; end if;
            for a in std_ulogic loop
                for b in std_ulogic loop
                    clk <= a; wait for 1 ns;
                    nr := rises; nf := falls;
                    clk <= b; wait for 1 ns;
                    if e = 1 and (a = '0' or a = 'L') and (b = '1' or b = 'H') then nr := nr + 1; end if;
                    if e = 1 and (a = '1' or a = 'H') and (b = '0' or b = 'L') then nf := nf + 1; end if;
                    assert nr = rises and nf = falls severity failure;
                end loop;
            end loop;
        end loop;
        report "PASS edge matrix"; std.env.finish;
    end process;
end;
