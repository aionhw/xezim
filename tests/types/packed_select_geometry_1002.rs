//! IEEE 1800-2023 §7.4.1, §11.5.1: packed selects use the complete declared
//! geometry of typedef locals and members of unpacked struct-array elements.

#[test]
fn reported_packed_selects_keep_their_element_widths() {
    let sim = xezim::simulate(
        include_str!("../issue_cases/packed_select_geometry_1002.sv"),
        100_000,
    )
    .expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "TEST_PASS"),
        "{:?}",
        sim.output
    );
    assert!(
        !sim.output.iter().any(|o| o.message.starts_with("FAIL:")),
        "{:?}",
        sim.output
    );
}

#[test]
fn local_typedef_chains_map_labels_and_check_every_index() {
    let sim = xezim::simulate(
        r#"
typedef logic [2:0] elem_t;
typedef elem_t [0:3][5:2] matrix_t;
typedef bit [0:3][5:2][2:0] bit_matrix_t;
module top;
 matrix_t data=48'h123456789abc;
 int calls;
 function automatic int next_row(); calls++; return 1; endfunction
 function automatic int next_col(); calls++; return 3; endfunction
 function automatic int bad_row(); calls++; return 4; endfunction
 function automatic elem_t read(input matrix_t source,input integer row,col);
   matrix_t tmp=source;
   return tmp[row][col];
 endfunction
 function automatic logic [2:0] read_bit(input bit_matrix_t source,input integer row,col);
   bit_matrix_t tmp=source;
   return tmp[row][col];
 endfunction
 function automatic elem_t read_effects(input matrix_t source,input bit invalid);
   matrix_t tmp=source;
   if(invalid) return tmp[bad_row()][next_col()];
   return tmp[next_row()][next_col()];
 endfunction
 initial begin
   logic [47:0] bits_value=48'h123456789abc;
   for(int row=0;row<4;row++) begin
     for(int col=2;col<=5;col++) begin
       if(read(data,row,col)!==((bits_value>>(((3-row)*4+col-2)*3))&3'h7))
         $fatal(1,"declared index geometry");
     end
   end
   if(read(data,0,6)!==3'bxxx || read(data,4,2)!==3'bxxx ||
      read(data,-1,2)!==3'bxxx || read(data,0,32'bx)!==3'bxxx)
     $fatal(1,"invalid index");
   if(read_bit('1,0,6)!==3'b000 || read_bit('1,0,32'bx)!==3'b000)
     $fatal(1,"two-state default");
   if(read_effects(data,0)!==((bits_value>>27)&3'h7) || calls!=2)
     $fatal(1,"selector side effects");
   if(read_effects(data,1)!==3'bxxx || calls!=4)
     $fatal(1,"invalid selector side effects");
   $display("T|PASS");
 end
endmodule
"#,
        1000,
    )
    .expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}
