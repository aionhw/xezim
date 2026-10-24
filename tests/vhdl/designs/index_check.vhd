entity index_check is end entity;
architecture a of index_check is begin
  process
    variable a : bit_vector(3 downto 0);
    variable i : integer := 9;
  begin
    a(i) := '1';
    wait;
  end process;
end architecture;
