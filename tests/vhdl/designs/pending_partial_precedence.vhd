-- IEEE 1076-2008 §10.5.2: projected waveforms are updated scalar by scalar.
entity pending_partial_precedence is end;
architecture a of pending_partial_precedence is
  signal q : bit_vector(2 downto 0) := "000";
  type words is array (0 to 1) of bit_vector(3 downto 0);
  signal r : words := (others => "0000");
begin
  process
  begin
    q <= "101"; q(0) <= '0';
    r(0) <= "1010"; r(0)(0) <= '1';
    wait for 0 ns;
    assert q = "100" and r(0) = "1011"
      report "leaf after word lost precedence" severity failure;
    q(2) <= '0'; q <= "011";
    r(0)(1) <= '1'; r(0) <= "0100";
    wait for 0 ns;
    assert q = "011" and r(0) = "0100"
      report "word after leaf lost precedence" severity failure;
    q(0) <= '0'; q(0) <= '1';
    r(1) <= "1111"; r(1)(0) <= '0'; r(0)(0) <= '1';
    wait for 0 ns;
    assert q = "011" and r(0) = "0101" and r(1) = "1110"
      report "overlapping or sibling pending writes crossed" severity failure;
    report "PASS pending partial precedence";
    std.env.finish;
  end process;
end;
