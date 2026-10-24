-- Rejected uses of guarded signals (§6.4.2.3, §7.4, §10.5.2.2, §11.6).
entity guarded_errs is end entity;
architecture a of guarded_errs is
  signal u : bit bus;
  signal s : integer;
  disconnect s : integer after 1 ns;
begin
  process begin s <= null; wait; end process;
  s <= guarded 1;
end architecture;
