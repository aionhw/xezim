entity shreg is
  generic (W : positive := 4; NAME : string := "dflt"; INIT : bit := '0');
  port (clk : in bit; en : in bit; d : in bit; q : out bit_vector(W - 1 downto 0); msb : out bit);
end entity;

architecture rtl of shreg is
  signal r : bit_vector(W - 1 downto 0) := (others => INIT);
  constant HALF : natural := W / 2;
begin
  process (clk)
  begin
    if clk'event and clk = '1' and en = '1' then
      r <= r(W - 2 downto 0) & d;
    end if;
  end process;
  q <= r;
  msb <= r(W - 1);
  process (r)
  begin
    report NAME & ": W=" & integer'image(W) & " half=" & integer'image(HALF) & " r=" & to_string(r) & " len=" & integer'image(r'length);
  end process;
end architecture;

entity generics is
end entity;

architecture sim of generics is
  signal clk, d : bit := '0';
  signal q8 : bit_vector(7 downto 0);
  signal q3 : bit_vector(2 downto 0);
  signal m8 : bit;
begin
  a : entity work.shreg generic map (W => 8, NAME => "eight") port map (clk => clk, en => '1', d => d, q => q8, msb => m8);
  b : entity work.shreg generic map (3, "three", '1') port map (clk, '1', d, q3, open);
  c : entity work.shreg port map (clk => clk, en => '0', d => d, q => open, msb => open);
  process
  begin
    for i in 0 to 9 loop
      d <= not d;
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    report "q8=" & to_string(q8) & " q3=" & to_string(q3) & " m8=" & bit'image(m8);
    wait;
  end process;
end architecture;
