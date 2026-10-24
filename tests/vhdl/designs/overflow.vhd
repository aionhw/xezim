entity overflow is end entity;
architecture a of overflow is begin
  process
    variable i : integer := integer'high;
  begin
    wait for 1 ns;
    i := i + 1;
    report "not reached";
    wait;
  end process;
end architecture;
