library ieee;
use ieee.std_logic_1164.all;

entity wide_logic is
    port (a : in std_logic_vector(0 to 68);
          y : out std_logic_vector(68 downto 0));
end entity;

architecture rtl of wide_logic is
begin
    -- Rotate by position. This crosses a machine-word boundary and checks
    -- the leftmost and rightmost bits independently of round-trip identity.
    y <= a(1 to 68) & a(0);
end architecture;
