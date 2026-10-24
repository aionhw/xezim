entity time_format is end entity;
architecture a of time_format is begin
  process begin
    wait for 1500 ps; report "a";
    wait for 999 fs; report "b";
    wait for 1 us; report "c";
    wait for 1 ms; report "d";
    wait;
  end process;
end architecture;
