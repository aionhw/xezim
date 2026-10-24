entity leaf is
  generic (n : integer := 0);
  port (a : in bit; y : out bit);
end entity;
architecture rtl of leaf is
begin
  process (a) begin
    report "leaf " & integer'image(n) & " a=" & bit'image(a);
    y <= not a;
  end process;
end architecture;

entity generates is end entity;
architecture tb of generates is
  signal s : bit_vector(0 to 3);
  signal t : bit_vector(0 to 3);
  signal x : bit := '0';
  signal xe, xc : bit;
begin
  gen: for i in 0 to 3 generate
    signal loc : bit;
  begin
    p: process (x) begin
      if now = 0 ns then report "gen proc " & integer'image(i); end if;
      s(i) <= x;
    end process;
    u: entity work.leaf generic map (n => i) port map (a => s(i), y => t(i));
  end generate;
  ig: if t'length = 4 generate
    process begin report "if-gen true"; wait; end process;
  elsif false generate
    process begin report "elsif"; wait; end process;
  else generate
    process begin report "else"; wait; end process;
  end generate;
  cg: case 2 generate
    when 1 => process begin report "case 1"; wait; end process;
    when others => process begin report "case others"; wait; end process;
  end generate;
  ue: entity work.leaf generic map (n => 10) port map (a => x, y => xe);
  process
  begin
    wait for 1 ns;
    x <= '1';
    wait for 1 ns;
    report "t=" & to_string(t) & " xe=" & bit'image(xe);
    wait;
  end process;

end architecture;
