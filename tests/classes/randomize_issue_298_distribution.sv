
// §18.5.10: the #298 class without its arrays. `!allow_cfg_space` caps
// total_bytes (and with it cmd.count) through a guarded dist, so the
// reference draws allow_cfg_space = 1 nearly always and cmd.count about
// uniformly; deciding it 50/50 left half the draws with count <= 3.
module randomize_issue_298_distribution ;

   typedef struct packed {
      logic         valid;
      logic         wr;
      logic [39:0]  base;
      logic [2:0]   src;
      logic [2:0]   sz;
      logic [7:0]   count;
      logic         priv;
      logic [1:0]   kind;
      logic         excl;
   } port_cmd_t;

   typedef struct packed {
      logic         valid;
      logic [127:0] data;
      logic [15:0]  mask;
      logic         is_last;
   } port128_wr_t;

   typedef struct packed {
      logic         valid;
      logic [127:0] data;
      logic         is_last;
      logic [1:0]   rsp;
   } port128_rd_t;

   typedef enum logic [1:0] { port_NONEXCL = 2'd0, port_EXCL = 2'd1 } port_excl_t ;

   class XPortTransfer #(
      type CMD_TYPE_T = port_cmd_t,
      type RD_TYPE_T = port128_rd_t,
      type WR_TYPE_T = port128_wr_t);

      rand bit [31:0] cmd_delay ;
      rand bit [31:0] wr_delay ;
      rand CMD_TYPE_T cmd ;
      rand bit [39:0] last_addr ;
      rand bit [13:0] total_bytes ;
      rand bit final_item_in_set ;
      rand bit disable_mask ;
      rand bit allow_cfg_space ;
      rand WR_TYPE_T wr_beats [] ;
      rand bit wait_for_rsp ;
      rand RD_TYPE_T rd_beats [] ;

      // State variables (non-rand)
      bit [ 3:0] max_cmd_src ;
      bit [12:0] max_cmd_bytes_m1 ;
      bit enable_circ, enable_excl ;
      bit [ 4:0] ident_bits ;
      bit [5:0] fixed_bpos_in_range ;
      bit block_reads, block_writes ;
      int PORT_BITS ;
      bit [2:0] max_cmd_sz ;
      int MASK_BITS ;
      WR_TYPE_T wr_samp ;

      constraint honor_max_state_vars_c {
         if (!allow_cfg_space) {
            total_bytes dist {
               [0:max_cmd_bytes_m1 >> 2] :/ 3,
               [max_cmd_bytes_m1 >> 2:max_cmd_bytes_m1] :/ 5,
               max_cmd_bytes_m1 + 1 :/ 2
            } ;
         }
         cmd.src <= max_cmd_src ;
         block_reads -> (cmd.wr == 1);
         block_writes -> (cmd.wr == 0);
      }
      constraint cmd_delays_c {
         cmd_delay dist { [0:15] :/ 900 , [16:56] :/ 80 , [57:63] :/ 20, [500:700] :/ 1 } ;
         wr_delay dist { [0:15] :/ 900 , [16:56] :/ 80 , [57:63] :/ 20, [200:500] :/ 1 } ;
      }
      constraint total_bytes_c {
         total_bytes == ((cmd.count + 1) << cmd.sz) ;
      }
      constraint port_sz_limitations_c { cmd.sz <= max_cmd_sz ; }
      constraint excl_limitations_c {
         if (enable_excl) {
            cmd.excl dist {  0 := 14, 1 := 1 } ;
            cmd.priv dist {  1 := 9, 0 := 1 } ;
         } else {
            cmd.excl == 0 ;
            cmd.priv == 0 ;
         }
         if (cmd.excl) {
            total_bytes <= 64 ;
            cmd.base[6] == last_addr[6] ;
         }
      }
      constraint excl_kind_limitations_c { (cmd.excl == 1) -> (cmd.kind == 1); }
      constraint kind_limitations_c {
         if (enable_circ) { cmd.kind dist { 1 := 9, 2 := 1 }; }
         else { (cmd.kind == 1); }
         if (cmd.kind == 2) { last_addr == cmd.base ; }
         else { (last_addr == (cmd.base + total_bytes - 1)) ; }
      }
      constraint circ_limitations_c {
         if (cmd.kind == 2) {
            cmd.wr == 0 ;
            cmd.excl == port_excl_t'(0) ;
            total_bytes inside { 32, 64, 128, 256 } ;
            cmd.sz == max_cmd_sz ;
         }
      }
      constraint no_page_crossing_c {
         (cmd.base >> fixed_bpos_in_range) == (last_addr >> fixed_bpos_in_range) ;
      }
      constraint beat_count_c {

      }
      constraint disable_mask_c { disable_mask dist { 0 := 99, 1 := 1 } ; }
      constraint wr_beats_c {
         foreach (wr_beats[k]) {
            wr_beats[k].valid == 1 ;
            if (disable_mask) {
               wr_beats[k].mask == (1 << MASK_BITS) - 1 ;
            } else {
               wr_beats[k].mask dist {
                  (1 << MASK_BITS) - 1 :/ 40 ,
                  [0 : (1 << MASK_BITS) - 2] :/ 60
               } ;
            }
         }
         foreach (wr_beats[k]) {
            if (k == (cmd.count)) { wr_beats[k].is_last == 1 ; }
            else { wr_beats[k].is_last == 0 ; }
         }
      }
      constraint align_constraints_c {
         (cmd.base & ((1 << cmd.sz) - 1)) == 0 ;
         (cmd.excl == 1) -> ((cmd.base & (total_bytes - 1)) == 0);
         cmd.count <= 255 ;
      }
      constraint solve_order_c {
         solve cmd.kind before cmd.count ;
         solve cmd.count before cmd.sz ;
         solve cmd.sz before total_bytes ;
         solve cmd.count before wr_beats ;
         solve cmd.kind, cmd.sz before cmd.base ;
         solve cmd.base, cmd.count before last_addr ;
      }

      function new (string name = "port_transfer_info") ;
         PORT_BITS = $bits(wr_samp.data);
         MASK_BITS = $bits(wr_samp.mask);
         max_cmd_sz = $clog2(PORT_BITS) - 3 ;
         fixed_bpos_in_range = 16 ;
      endfunction : new
   endclass : XPortTransfer

   // helper: build a transfer with the state vars the config seqs set before randomize
   function automatic XPortTransfer mk_cfg_cmd();
      XPortTransfer tr = new ;
      tr.max_cmd_src = 0 ;
      tr.enable_circ = 0 ;
      tr.ident_bits = 17 ;
      tr.enable_excl = 0 ;
      tr.max_cmd_bytes_m1 = 3 ;
      return tr ;
   endfunction

   initial begin
      XPortTransfer tr = mk_cfg_cmd() ;
      int fails = 0, bad = 0, acs = 0, low = 0;
      // Scalar-only probe; array coverage: issue_298_size_coupled_struct_arrays
      // and size_phase_class_shapes in randomize_issues_298_299.rs.
      tr.wr_beats.rand_mode(0);
      tr.rd_beats.rand_mode(0);
      for (int it = 0; it < 100; it++) begin
         if (tr.randomize() == 0) fails++ ;
         else begin
            if (tr.total_bytes != ((tr.cmd.count + 1) << tr.cmd.sz)) bad++ ;
            if (tr.last_addr != tr.cmd.base + tr.total_bytes - 1) bad++ ;
            if ((tr.cmd.base >> 16) != (tr.last_addr >> 16)) bad++ ;
            if ((tr.cmd.base & ((1 << tr.cmd.sz) - 1)) != 0) bad++ ;
            acs += tr.allow_cfg_space ;
            low += tr.cmd.count < 32 ;
         end
      end
      $display("T|fails=%0d bad=%0d acs=%0d low=%0d", fails, bad, acs, low);
   end
endmodule
