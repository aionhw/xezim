entity directives is end entity;
architecture a of directives is
begin
`warning "check the configuration"
`if TOOL_TYPE /= "SIMULATION" then
`error "not taken"
`else
  `error "this design needs a synthesis tool"
`end if
end architecture;
