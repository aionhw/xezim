entity adder is
  generic (w : integer := 4; tag : string := "add");
  port (a, b : in integer; s : out integer; extra : in bit := '1');
end entity;
architecture plus of adder is
begin
  s <= a + b;
  process begin report tag & " plus w=" & integer'image(w) & " extra=" & bit'image(extra); wait; end process;
end architecture;
architecture minus of adder is
begin
  s <= a - b;
  process begin report tag & " minus w=" & integer'image(w); wait; end process;
end architecture;

entity sub is
  port (x : in integer; y : out integer);
end entity;
architecture rtl of sub is
  component adder is
    generic (w : integer := 8; tag : string := "cmp");
    port (a, b : in integer; s : out integer);
  end component;
begin
  u: adder port map (a => x, b => 1, s => y);
end architecture;

configuration sub_cfg of sub is
  for rtl
    for u : adder use entity work.adder(minus) generic map (w => 99, tag => "cfg"); end for;
  end for;
end configuration;

entity components is end entity;
architecture tb of components is
  component adder is
    generic (w : integer := 2; tag : string := "top");
    port (a, b : in integer; s : out integer);
  end component;
  component sub is port (x : in integer; y : out integer); end component;
  component nothing is port (x : in integer); end component;
  signal p, q, r1, r2, r3, r4, r5 : integer := 0;
  for u2 : adder use entity work.adder(minus) generic map (w => 7, tag => tag);
  for others : adder use entity work.adder(plus) generic map (w => w * 10) port map (a => a, b => b, s => s, extra => '0');
begin
  u1: adder port map (p, q, r1);
  u2: adder generic map (tag => "u2") port map (a => p, b => q, s => r2);
  u3: adder generic map (5) port map (a => p, b => q, s => r3);
  us: sub port map (x => p, y => r4);
  un: nothing port map (x => p);
  ue: configuration work.sub_cfg port map (x => p, y => r5);
  process
  begin
    p <= 10; q <= 3;
    wait for 1 ns;
    report "r1=" & integer'image(r1) & " r2=" & integer'image(r2) & " r3=" & integer'image(r3) & " r4=" & integer'image(r4) & " r5=" & integer'image(r5);
    wait;
  end process;
end architecture;

