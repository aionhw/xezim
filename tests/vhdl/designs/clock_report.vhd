entity clock_report is end entity;
architecture a of clock_report is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 20 ns;
  process (clk) begin report "clk=" & bit'image(clk); end process;
end architecture;
