entity assert_failure is end entity;
architecture a of assert_failure is begin
  process begin
    assert false report "fatal assert" severity failure;
    wait;
  end process;
end architecture;
