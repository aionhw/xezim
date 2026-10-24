-- Guarded blocks (§11.2) and guarded assignments (§11.6): GUARD from the
-- guard expression (also of an enclosing block), guarded signals of kind
-- bus and register (§6.4.2.3) of scalar and array subtypes, disconnection
-- specifications (§7.4) and null transactions turning drivers off; a bus
-- with no driver on resolves an empty array, a register keeps its value.
library ieee; use ieee.std_logic_1164.all;
entity guarded_blocks is end entity;
architecture a of guarded_blocks is
  signal clk, en1, en2 : bit := '0';
  signal d, q : integer := 0;
  function res(v : bit_vector) return bit is
  begin
    for i in v'range loop if v(i) = '1' then return '1'; end if; end loop;
    return '0';
  end function;
  subtype rbit is res bit;
  signal busb : rbit bus;
  signal reg : rbit register;
  signal sl : std_logic bus;
  disconnect busb : rbit after 3 ns;
  signal sel : integer := 0;
  signal en : std_logic := '0';
  signal x, y : std_logic_vector(3 downto 0) := "0000";
  signal bv : std_logic_vector(3 downto 0) bus;
  signal r : std_logic_vector(3 downto 0) register := "1010";
  signal o : integer := 0;
  disconnect bv : std_logic_vector after 2 ns;
begin
  process
  begin
    d <= 5; wait for 5 ns;
    clk <= '1'; wait for 5 ns; clk <= '0'; d <= 6; wait for 5 ns;
    clk <= '1'; wait for 5 ns; clk <= '0';
    en1 <= '1'; wait for 10 ns;
    en2 <= '1'; wait for 10 ns;
    en1 <= '0'; wait for 10 ns;
    en2 <= '0'; wait for 10 ns;
    x <= "0101"; y <= "0011"; wait for 5 ns;
    sel <= 1; wait for 5 ns;
    sel <= 2; wait for 5 ns;
    en <= '1'; wait for 5 ns;
    x <= "1111"; wait for 5 ns;
    en <= '0'; wait for 5 ns;
    en <= '1'; wait for 5 ns;
    sel <= 0; wait for 10 ns;
    wait;
  end process;
  b1: block (clk = '1' and not clk'stable) begin
    q <= guarded d;
  end block;
  b2: block (en1 = '1') begin
    busb <= guarded '1';
    reg <= guarded '1';
    sl <= guarded 'H';
  end block;
  b3: block (en2 = '1')
  begin
    busb <= guarded '0' after 1 ns;
    reg <= guarded '0';
    sl <= guarded '0';
  end block;
  outer: block (sel /= 0)
  begin
    bv <= guarded x when sel = 1 else y;
    inner: block (guard and en = '1') begin
      r <= guarded x;
      o <= guarded 7 when x = "1111" else 3;
    end block;
  end block;
  process
  begin
    report "q=" & integer'image(q) & " bus=" & bit'image(busb) & " reg=" & bit'image(reg)
      & " sl=" & std_logic'image(sl) & " bv=" & to_string(bv) & " r=" & to_string(r)
      & " o=" & integer'image(o);
    wait on q, busb, reg, sl, bv, r, o;
  end process;
end architecture;
