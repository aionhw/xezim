library ieee; use ieee.std_logic_1164.all;
entity full_add is
  port (a, b, ci : in std_logic; s, co : out std_logic);
end entity;
architecture gates of full_add is
begin
  s  <= a xor b xor ci;
  co <= (a and b) or (ci and (a xor b));
end architecture;
architecture behav of full_add is
begin
  process (a, b, ci)
    variable n : integer;
  begin
    n := 0;
    if a = '1' then n := n + 1; end if;
    if b = '1' then n := n + 1; end if;
    if ci = '1' then n := n + 1; end if;
    if n mod 2 = 1 then s <= '1'; else s <= '0'; end if;
    if n >= 2 then co <= '1'; else co <= '0'; end if;
  end process;
end architecture;

library ieee; use ieee.std_logic_1164.all;
entity adder_n is
  generic (n : positive := 4);
  port (a, b : in std_logic_vector(n-1 downto 0); ci : in std_logic;
        s : out std_logic_vector(n-1 downto 0); co : out std_logic);
end entity;
architecture ripple of adder_n is
  component full_add is port (a, b, ci : in std_logic; s, co : out std_logic); end component;
  signal c : std_logic_vector(n downto 0);
begin
  c(0) <= ci;
  bits: for i in 0 to n-1 generate
    lsb: if i = 0 generate
      fa: full_add port map (a => a(i), b => b(i), ci => c(i), s => s(i), co => c(i+1));
    else generate
      fa: full_add port map (a(i), b(i), c(i), s(i), c(i+1));
    end generate;
  end generate;
  co <= c(n);
end architecture;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity rca_tb is end entity;
architecture tb of rca_tb is
  component adder_n is
    generic (n : positive := 4);
    port (a, b : in std_logic_vector(n-1 downto 0); ci : in std_logic;
          s : out std_logic_vector(n-1 downto 0); co : out std_logic);
  end component;
  constant W : positive := 6;
  signal a, b, s : std_logic_vector(W-1 downto 0) := (others => '0');
  signal ci, co : std_logic := '0';
  signal errors : natural := 0;
begin
  dut: adder_n generic map (n => W) port map (a, b, ci, s, co);
  stim: process
    variable expect : unsigned(W downto 0);
    variable seed : natural := 7;
  begin
    for k in 0 to 39 loop
      seed := (seed * 1103 + 12345) mod 65536;
      a <= std_logic_vector(to_unsigned(seed mod 2**W, W));
      b <= std_logic_vector(to_unsigned((seed / 64) mod 2**W, W));
      if (seed / 4096) mod 2 = 1 then ci <= '1'; else ci <= '0'; end if;
      wait for 10 ns;
      expect := resize(unsigned(a), W+1) + resize(unsigned(b), W+1);
      if ci = '1' then expect := expect + 1; end if;
      if co & s /= std_logic_vector(expect) then
        report "mismatch " & to_string(a) & " + " & to_string(b) & " + " & std_logic'image(ci) & " = " & to_string(co & s) severity error;
        errors <= errors + 1;
      elsif k mod 8 = 0 then
        report to_string(a) & " + " & to_string(b) & " + " & std_logic'image(ci) & " = " & to_string(co & s);
      end if;
    end loop;
    wait for 1 ns;
    report "done, errors=" & integer'image(errors);
    wait;
  end process;
end architecture;

configuration ripple_adder of rca_tb is
  for tb
    for dut : adder_n
      use entity work.adder_n(ripple);
      for ripple
        for bits(0 to 2)
          for lsb
            for fa : full_add use entity work.full_add(gates); end for;
          end for;
        end for;
      end for;
    end for;
  end for;
end configuration;
