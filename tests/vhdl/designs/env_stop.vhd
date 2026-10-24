use std.env.all;
entity env_stop is end entity;
architecture a of env_stop is begin
  process begin
    wait for 5 ns;
    report "stopping";
    stop;
    report "after stop";
    wait;
  end process;
end architecture;
