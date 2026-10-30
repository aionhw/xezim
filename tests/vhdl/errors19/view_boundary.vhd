package vb_pkg is
  type bus_t is record
    req, ack : bit;
  end record;
  view master of bus_t is
    req : out;
    ack : in;
  end view;
end package;
use work.vb_pkg.all;
entity vb_child is
  port (m : view master);
end entity;
architecture a of vb_child is
begin
  m.req <= m.ack;
end architecture;
