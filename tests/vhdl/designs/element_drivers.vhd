-- Elements of resolved array signals driven by processes of their own
-- (§14.7.2, §14.7.3.2): each leaf has one source, which RESOLVED returns,
-- and the update copies each driver's elements. One element of `bus_s` has
-- two sources, resolved by the table. SHIFT_LEFT, XOR and a loop over the
-- array in a clocked process complete a typical register file.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity element_drivers is end entity;
architecture a of element_drivers is
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  type arr is array (0 to 7) of unsigned(7 downto 0);
  signal r, c : arr;
  signal acc : unsigned(7 downto 0);
  signal bus_s : std_logic_vector(3 downto 0);
begin
  clk <= not clk after 5 ns;
  rst <= '0' after 20 ns;
  reg0 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(0) <= to_unsigned(3, 8);
      else r(0) <= c(0) + r(1); end if;
    end if;
  end process;
  comb0 : process (r) begin
    c(0) <= (r(0) xor shift_left(r(0), 3)) + 0;
  end process;
  reg1 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(1) <= to_unsigned(32, 8);
      else r(1) <= c(1) + r(2); end if;
    end if;
  end process;
  comb1 : process (r) begin
    c(1) <= (r(1) xor shift_left(r(1), 3)) + 1;
  end process;
  reg2 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(2) <= to_unsigned(61, 8);
      else r(2) <= c(2) + r(3); end if;
    end if;
  end process;
  comb2 : process (r) begin
    c(2) <= (r(2) xor shift_left(r(2), 3)) + 2;
  end process;
  reg3 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(3) <= to_unsigned(90, 8);
      else r(3) <= c(3) + r(4); end if;
    end if;
  end process;
  comb3 : process (r) begin
    c(3) <= (r(3) xor shift_left(r(3), 3)) + 3;
  end process;
  reg4 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(4) <= to_unsigned(119, 8);
      else r(4) <= c(4) + r(5); end if;
    end if;
  end process;
  comb4 : process (r) begin
    c(4) <= (r(4) xor shift_left(r(4), 3)) + 4;
  end process;
  reg5 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(5) <= to_unsigned(148, 8);
      else r(5) <= c(5) + r(6); end if;
    end if;
  end process;
  comb5 : process (r) begin
    c(5) <= (r(5) xor shift_left(r(5), 3)) + 5;
  end process;
  reg6 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(6) <= to_unsigned(177, 8);
      else r(6) <= c(6) + r(7); end if;
    end if;
  end process;
  comb6 : process (r) begin
    c(6) <= (r(6) xor shift_left(r(6), 3)) + 6;
  end process;
  reg7 : process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then r(7) <= to_unsigned(206, 8);
      else r(7) <= c(7) + r(0); end if;
    end if;
  end process;
  comb7 : process (r) begin
    c(7) <= (r(7) xor shift_left(r(7), 3)) + 7;
  end process;
  sum : process (clk)
    variable v : unsigned(7 downto 0);
  begin
    if rising_edge(clk) then
      v := (others => '0');
      for i in r'range loop v := v xor r(i); end loop;
      acc <= v;
    end if;
  end process;
  bus_s(3 downto 1) <= "10Z";
  bus_s(1) <= '1' when acc(4) = '1' else 'Z';
  bus_s(0) <= acc(6);
  done : process
  begin
    for k in 1 to 6 loop
      wait for 20 ns;
      report "acc=" & integer'image(to_integer(acc)) & " r0=" & integer'image(to_integer(r(0))) &
        " r7=" & integer'image(to_integer(r(7))) & " bus=" & to_string(bus_s);
    end loop;
    std.env.finish;
  end process;
end architecture;
