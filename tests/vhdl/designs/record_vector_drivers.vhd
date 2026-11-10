-- Whole-vector assignments to fields of a record signal, mixed with element
-- assignments in the same cycle and with delayed transactions (§10.5.2.2,
-- §14.7.3.2); `(others => x)` aggregates (§9.3.3.3).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity record_vector_drivers is
end entity;

architecture sim of record_vector_drivers is
  type reg_t is record
    value : std_logic_vector(15 downto 0);
    flag  : std_logic;
    cnt   : unsigned(7 downto 0);
  end record;
  type regs_t is record
    a : reg_t;
    b : reg_t;
  end record;
  signal r   : regs_t;
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal fill : std_logic := '0';
begin
  clk <= not clk after 5 ns;

  -- One process per field group, as a register block has.
  pa : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r.a.value <= (others => '0');
        r.a.flag  <= '0';
        r.a.cnt   <= (others => '0');
      else
        r.a.value <= (others => fill);
        r.a.value(3) <= '1';              -- after the whole vector: wins
        r.a.cnt   <= r.a.cnt + 1;
        r.a.flag  <= not r.a.flag;
      end if;
    end if;
  end process;

  pb : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r.b.value <= x"00FF";
        r.b.cnt   <= x"80";
        r.b.flag  <= '1';
      else
        r.b.value(0) <= '0';              -- before the whole vector: lost
        r.b.value <= r.b.value(14 downto 0) & r.a.flag;
        if r.b.cnt = x"82" then
          r.b.cnt <= x"F0" after 3 ns;    -- queued, then whole again below
        elsif r.b.cnt = x"F0" then
          r.b.cnt <= (others => '1');
          r.b.cnt(7 downto 4) <= "0101";
        else
          r.b.cnt <= r.b.cnt + 1;
        end if;
      end if;
    end if;
  end process;

  stim : process
  begin
    wait for 12 ns;
    rst <= '0';
    for i in 1 to 8 loop
      wait until falling_edge(clk);
      report "a=" & to_hstring(r.a.value) & " " & std_logic'image(r.a.flag) & " " & to_hstring(r.a.cnt)
           & " b=" & to_hstring(r.b.value) & " " & std_logic'image(r.b.flag) & " " & to_hstring(r.b.cnt);
      if i = 3 then fill <= '1'; end if;
      if i = 6 then fill <= 'Z'; end if;
    end loop;
    std.env.finish;
  end process;
end architecture;
