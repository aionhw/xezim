entity cell is
  generic (id : integer := -1);
  port (i : in bit; o : out bit);
end entity;
architecture buf of cell is
begin
  o <= i;
  process begin report "buf " & integer'image(id); wait; end process;
end architecture;
architecture inv of cell is
begin
  o <= not i;
  process begin report "inv " & integer'image(id); wait; end process;
end architecture;

entity row is
  generic (n : integer := 2);
  port (i : in bit; o : out bit);
end entity;
architecture rtl of row is
  component cell is generic (id : integer := 0); port (i : in bit; o : out bit); end component;
  signal chain : bit_vector(0 to n);
begin
  chain(0) <= i;
  g: for k in 0 to n-1 generate
    c: cell generic map (id => k) port map (i => chain(k), o => chain(k+1));
  end generate;
  o <= chain(n);
end architecture;

entity config_tb is end entity;
architecture tb of config_tb is
  component row is generic (n : integer := 2); port (i : in bit; o : out bit); end component;
  component cell is generic (id : integer := 0); port (i : in bit; o : out bit); end component;
  signal a, y1, y2 : bit := '0';
begin
  r1: row generic map (n => 3) port map (i => a, o => y1);
  sel: if true generate
    c0: cell generic map (id => 100) port map (i => a, o => y2);
  end generate;
  process
  begin
    wait for 1 ns;
    report "y1=" & bit'image(y1) & " y2=" & bit'image(y2);
    a <= '1';
    wait for 1 ns;
    report "y1=" & bit'image(y1) & " y2=" & bit'image(y2);
    wait;
  end process;
end architecture;

configuration config_decl of config_tb is
  for tb
    for r1 : row
      use entity work.row(rtl);
      for rtl
        for g(1)
          for c : cell use entity work.cell(inv) generic map (id => id + 10); end for;
        end for;
        for g(0 to 0)
          for all : cell use entity work.cell(buf); end for;
        end for;
      end for;
    end for;
    for sel
      for c0 : cell use entity work.cell(inv); end for;
    end for;
  end for;
end configuration;
