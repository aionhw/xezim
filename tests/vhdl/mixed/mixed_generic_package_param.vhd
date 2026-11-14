library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity gen_ent is
  generic (Name  : string  := "default";
           Width : integer := 1);
  port (o : out std_logic;
        w : out std_logic_vector(7 downto 0));
end entity;

architecture a of gen_ent is
begin
  o <= '1' when Name = "from_pkg" else '0';
  w <= std_logic_vector(to_unsigned(Width, 8));
end architecture;
