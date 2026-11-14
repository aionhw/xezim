library ieee;
use ieee.std_logic_1164.all;

-- Ports of mode inout: a scalar and a vector the entity drives while `oe`
-- is '1', one bit it pulls low weakly, and one it never uses.
entity io_ent is
  port (oe    : in    std_logic;
        dout  : in    std_logic_vector(7 downto 0);
        d     : inout std_logic;
        bus8  : inout std_logic_vector(7 downto 0);
        nc    : inout std_logic;
        seen  : out   std_logic;
        seen8 : out   std_logic_vector(7 downto 0));
end entity;

architecture a of io_ent is
begin
  d     <= dout(0) when oe = '1' else 'Z';
  bus8  <= dout when oe = '1' else (others => 'Z');
  bus8(7) <= 'L';
  seen  <= d;
  seen8 <= bus8;
end architecture;
