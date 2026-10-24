use std.env.all;
entity env_finish is end entity;
architecture a of env_finish is begin
  process begin wait for 5 ns; finish; end process;
end architecture;
