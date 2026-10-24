library ieee; use ieee.std_logic_1164.all;
entity blocks is end entity;
architecture tb of blocks is
  constant W : integer := 4;
  signal clk : std_logic := '0';
  signal q : std_logic_vector(W-1 downto 0) := (others => '0');
  signal done : boolean := false;
  function inv(x : std_logic) return std_logic is begin return not x; end function;
begin
  clk <= not clk after 5 ns when not done;
  blk: block
    generic (n : integer);
    generic map (n => W);
    port (c : in std_logic; o : out std_logic_vector(n-1 downto 0));
    port map (c => clk, o => q);
    signal cnt : integer range 0 to 2**n - 1 := 0;
  begin
    process (c) begin
      if rising_edge(c) then
        cnt <= (cnt + 1) mod 2**n;
      end if;
    end process;
    gen: for i in 0 to n-1 generate
      constant k : integer := i * 10;
      signal b : std_logic;
    begin
      b <= '1' when (cnt / 2**i) mod 2 = 1 else '0';
      o(i) <= b;
      inner: if i mod 2 = 0 generate
        process (b) begin
          if b'event and now > 0 ns then report "even bit " & integer'image(i) & " k=" & integer'image(k) & " -> " & std_logic'image(b) & " inv=" & std_logic'image(inv(b)); end if;
        end process;
      end generate;
    end generate;
  end block;
  process
  begin
    wait for 52 ns;
    report "q=" & to_string(q);
    done <= true;
    wait;
  end process;
end architecture;
