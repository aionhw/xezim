library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

-- Drivers of whole elements of arrays of vectors next to drivers of their
-- bits and slices: the later assignment of a process wins, each process
-- drives its own elements, and delayed transactions keep their order
-- (IEEE 1076-2008 §10.5.2.2, §14.7.2).
entity array_word_drivers is end;

architecture a of array_word_drivers is
  type warr is array (0 to 7) of std_logic_vector(31 downto 0);
  type m2 is array (0 to 3, 0 to 1) of std_logic_vector(7 downto 0);
  type rec is record
    v : std_logic_vector(31 downto 0);
    b : std_logic;
  end record;
  type rarr is array (0 to 7) of rec;
  signal w  : std_logic_vector(99 downto 0) := (others => '0');
  signal a1 : warr := (others => (others => '0'));
  signal a2 : warr := (others => (others => '0'));
  signal a3 : warr := (others => (others => '0'));
  signal mm : m2 := (others => (others => (others => '0')));
  signal rr : rarr;
  signal cat : std_logic_vector(255 downto 0);
  signal clk : std_logic := '0';
  signal ev : natural := 0;
  function h(v : std_logic_vector) return string is
  begin
    return to_hstring(v);
  end;
begin
  clk <= not clk after 5 ns when now < 20 ns else clk;

  -- wide vector: whole, slice, bit
  pw : process (clk)
  begin
    if rising_edge(clk) then
      w <= not w;
      w(15 downto 8) <= x"3C";
      w(99) <= '1';
    end if;
  end process;

  -- element ordering inside one process
  p_a1 : process
    variable i : integer := 2;
  begin
    a1(1) <= x"11111111";         -- word
    a1(1)(0) <= '0';              -- bit after word: bit wins
    a1(i)(4) <= '1';              -- bit first
    a1(i) <= x"22222222";         -- word after bit: word wins
    a1(3) <= x"33333333";
    a1(3)(31) <= '1';
    a1(3) <= x"0000000F";         -- word, bit, word
    a1(4) <= x"44444444" after 3 ns;
    a1(5) <= x"55555555";
    wait for 1 ns;
    a1(4) <= x"4A4A4A4A";         -- zero delay over a pending transaction
    a1(5)(7 downto 0) <= x"EE";
    wait for 5 ns;
    a1(6) <= x"66666666" after 2 ns, x"67676767" after 4 ns;
    a1(6)(3 downto 0) <= x"9" after 3 ns;
    wait for 10 ns;
    for k in 0 to 7 loop
      a1(k) <= std_logic_vector(to_unsigned(k * 17, 32));
    end loop;
    a1(7)(31 downto 28) <= "ZZZZ";
    wait;
  end process;

  -- two processes, different elements of one signal
  p_a2a : process (clk)
  begin
    if rising_edge(clk) then
      a2(0) <= std_logic_vector(unsigned(a2(0)) + 1);
      a2(2) <= not a2(2);
    end if;
  end process;
  p_a2b : process (clk)
  begin
    if falling_edge(clk) then
      a2(1) <= std_logic_vector(unsigned(a2(1)) + 3);
      a2(7)(15 downto 0) <= x"BEEF";
    end if;
  end process;

  -- two-dimensional
  p_mm : process
  begin
    mm(1, 0) <= x"A1";
    mm(3, 1) <= x"B7";
    mm(2, 1)(3) <= '1';
    wait for 2 ns;
    for i in 0 to 3 loop
      for j in 0 to 1 loop
        mm(i, j) <= std_logic_vector(to_unsigned(i * 16 + j, 8));
      end loop;
    end loop;
    mm(0, 1)(7) <= '1';
    wait;
  end process;

  -- array of records and a wide concatenation
  p_rr : process (clk)
  begin
    if rising_edge(clk) then
      for i in 0 to 7 loop
        rr(i).v <= std_logic_vector(to_unsigned(i + ev, 32));
        rr(i).b <= '1';
      end loop;
      rr(5).v(0) <= 'H';
      ev <= ev + 1;
    end if;
  end process;
  g : for i in 0 to 7 generate
    cat(32 * i + 31 downto 32 * i) <= rr(i).v;
  end generate;

  -- same element assigned as word in a3 through a variable index twice
  p_a3 : process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then
      a3(n mod 8) <= x"0000" & std_logic_vector(to_unsigned(n, 16));
      a3((n + 1) mod 8) <= x"FFFF" & std_logic_vector(to_unsigned(n, 16));
      a3(n mod 8) <= x"AAAA" & std_logic_vector(to_unsigned(n, 16));
      n := n + 3;
    end if;
  end process;

  mon : process
    variable l : line;
  begin
    wait for 500 ps;
    for t in 0 to 4 loop
      write(l, string'("@") & time'image(now));
      write(l, string'(" w=") & h(w(99 downto 92)) & h(w(31 downto 0)));
      for k in 0 to 7 loop
        write(l, string'(" ") & h(a1(k)));
      end loop;
      writeline(output, l);
      write(l, string'("  a2=") & h(a2(0)) & " " & h(a2(1)) & " " & h(a2(2)) & " " & h(a2(7)));
      write(l, string'(" mm="));
      for i in 0 to 3 loop
        write(l, h(mm(i, 0)) & h(mm(i, 1)) & " ");
      end loop;
      write(l, string'("rr=") & h(rr(0).v) & std_logic'image(rr(0).b) & h(rr(5).v) & h(rr(7).v));
      write(l, string'(" cat=") & h(cat(255 downto 224)) & h(cat(191 downto 160)));
      write(l, string'(" a3="));
      for k in 0 to 7 loop
        write(l, h(a3(k)(31 downto 12)) & " ");
      end loop;
      writeline(output, l);
      wait for 5 ns;
    end loop;
    wait;
  end process;
end;
