//! #255 (reported by rharikrishna25): a configuration object with rand
//! sub-objects in an object array, packed-struct rand members up to ~3800
//! bits wide constrained field by field from the parent, `solve ... before`
//! on sub-object properties and 40-bit arithmetic. The first test checks
//! TEST_PASS; the second test restates every constraint procedurally
//! and checks the solution of each randomize() call (§18.6.1: never 1 with a
//! constraint violated).

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with('['))
        .collect()
}

#[test]
fn issue_255_reproducer_passes() {
    let src = format!("{CLASSES}{REPRO}");
    let out = run(&src);
    assert!(out.iter().any(|m| m == "TEST_PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.starts_with("FAIL")), "{out:?}");
}

#[test]
fn issue_255_every_constraint_holds() {
    let src = format!("{CLASSES}{CHECK}");
    let out = run(&src);
    assert_eq!(
        out,
        [
            "s02 ok=1 bad=0 num=1",
            "s02b ok=1 bad=0",
            "s03 ok=1 bad=0 ds=1 num=1",
        ],
        "{out:?}"
    );
}

const CLASSES: &str = r#"
// ----- TB-common types -----
typedef logic [3:0]  u4_t;
typedef logic [4:0]  u5_t;
typedef logic [6:0]  u6_t;
typedef logic [7:0]  u8_t;
typedef logic [15:0] u16_t;
typedef logic [23:0] u24_t;
typedef logic [31:0] u32_t;
typedef logic [39:0] u40_t;
typedef int IPW14_t [] ;

// ----- parameters -----
`define NUM_IPW50    2
`define NUM_TOTAL_IPW21    17
`define NUM_TOTAL_IPW06 37
`define NUM_IPW60_IPW29S  4
`define MAX_OUTSTANDING_IPW50_RDATA_LANES 256
`define IPW13_WDATA_MAX_CREDIT 32

// ----- IPW02.sv:417 IPW50IPW32Registers (verbatim) -----
class XIPW50IPW32Registers ;
   int IPW09_ra ;
   int IPW10_ra ;
   int IPW18_en_ra ;
   int IPW23_IPW56_ra ;
   int IPW07_size_ra ;
   logic [9:0] IPW10_per_IPW55_per_die_per_x16 [9] ;

   function new();
      IPW10_per_IPW55_per_die_per_x16 = '{2,3,4,6,8,12,16,24,32};
      IPW09_ra = -1 ;
      IPW10_ra = -1 ;
      IPW18_en_ra = -1 ;
      IPW23_IPW56_ra = -1 ;
      IPW07_size_ra = -1 ;
   endfunction: new

   rand bit [1:0] IPW09_ds ;
   rand bit [3:0] IPW10_ds ;
   rand bit       IPW18_en_ds ;
   rand bit       IPW23_IPW56_ds ;
   rand bit       IPW07_size_ds ;

   constraint __from_IPW47_arg {
      IPW09_ds dist { 0 := 1, 1 := 49 };
      IPW10_ds inside { [0:8] } ;
      IPW18_en_ds inside { [0:1] };
      IPW23_IPW56_ds inside { [0:1] };
      IPW07_size_ds inside { [0:1] };
   }
   constraint __from_runarg {
      if (IPW09_ra != -1) { IPW09_ds == IPW09_ra ; }
      if (IPW10_ra != -1) { IPW10_ds == IPW10_ra ; }
      if (IPW18_en_ra != -1) { IPW18_en_ds == IPW18_en_ra ; }
      if (IPW23_IPW56_ra != -1) { IPW23_IPW56_ds == IPW23_IPW56_ra ; }
      if (IPW07_size_ra != -1) { IPW07_size_ds == IPW07_size_ra ; }
   }
endclass : XIPW50IPW32Registers

// ----- register struct mirrors (IPW47_sat_*_ctrl_registers.h, real widths) -----
typedef struct packed {
   bit [3:0]  IPW24_IPW33 ;   // u4
   bit [10:0] IPW19_IPW33 ;  // u11
   bit [8:0]  IPW20_IPW33 ;  // u9
   bit [7:0]  IPW11_IPW33 ; // u8
} x_IPW12_IPW33_t ;

typedef struct packed {
   bit [1:0] IPW47_IPW40_TYPE ;   // u2
   bit [1:0] IPW47_IPW07_SIZE ; // u2
   bit [1:0] IPW09 ;         // u2
   bit [1:0] CIPW52_GATING_EN ;   // u2
   bit [3:0] NUM_IPW55S ;         // u4
   bit [3:0] FIRST_IPW55 ;        // u4
} x_IPW56_t ;

typedef struct packed {
   x_IPW56_t                    REG_IPW47_IPW56_DUP ;
   bit [31:0]                   REG_IPW47_IPW19_IPW62 ;
   bit [(`NUM_TOTAL_IPW21*18)-1:0] REG_IPW47_IPW19_IPW34 ;
   x_IPW12_IPW33_t         REG_IPW47_POST_RD_IPW12_IPW33 ;
   x_IPW12_IPW33_t         REG_IPW47_POST_WR_IPW12_IPW33 ;
   bit [31:0]                   REG_IPW47_IPW08_CNTS ;
   bit [24:0]                   REG_IPW13_INTF_THRESHOLDS ;
   bit [63:0][23:0]             REG_IPW47_IPW60_SEG_LIMIT ;
   bit [63:0][23:0]             REG_IPW47_IPW60_SEG_RELOC ;
   bit [63:0][3:0]              REG_IPW47_IPW60_SEG_RIGHTS ;
   bit [23:0]                   REG_IPW47_IPW60_CLIENT_IPW29 ;
} x_IPW15_regs_t ;

typedef struct packed {
   x_IPW56_t                         REG_IPW47_IPW56 ;
   bit [(`NUM_TOTAL_IPW21*32)-1:0] REG_IPW47_IPW63_IPW25_THRESHOLDS ;
} x_IPW04_regs_t ;

// ----- IPW02.sv:613 IPW47IPW55CtrlConfig (constraints verbatim) -----
class XIPW47IPW55CtrlConfig ;
   rand x_IPW15_regs_t IPW15_r ;
   IPW14_t IPW14_raw ;
   bit cov_tweaks ;
   bit [`NUM_TOTAL_IPW21-1:0] IPW21_iterator ;
   // runarg-style IPW49e (createRunargConstraints pattern, 2 representative)
   int RUNARG_IPW19_IPW62 = -1 ;
   int RUNARG_IPW08_CNTS = -1 ;

   function new();
      cov_tweaks = $test$plusargs("cov_tweaks");
   endfunction: new

   constraint c_IPW19_IPW62 {
      IPW15_r.REG_IPW47_IPW19_IPW62 > 0 ;
      if (cov_tweaks) {
         IPW15_r.REG_IPW47_IPW19_IPW62 dist {
            0 :/ 2,
            [32'h1:32'h1851eb85] :/ 1,
            [32'h1851eb86:32'h48f5c28f] :/ 1,
            [32'h48f5c290:32'h79999998] :/ 1,
            [32'h79999999:32'h86666665] :/ 2,
            [32'h86666666:32'hb70a3d6f] :/ 1,
            [32'hb70a3d70:32'he7ae1479] :/ 1,
            [32'he7ae147a:32'hfffffffe] :/ 1,
            32'hffffffff :/ 2
         };
      } else {
         IPW15_r.REG_IPW47_IPW19_IPW62 dist {
            [1:10] :/ 1 , [11:100] :/ 9 , [101:1000] :/ 30 ,
            [1000:30000] :/ 40 , [30000:60000] :/ 10
         } ;
      }
   }
   constraint c_IPW19_IPW34s_and_counters {
      foreach (IPW21_iterator[i]) {
         IPW15_r.REG_IPW47_IPW19_IPW34[(i*18) +  8] dist { 1 := 9, 0 := 1 };
         IPW15_r.REG_IPW47_IPW19_IPW34[(i*18) + 17] dist { 1 := 9, 0 := 1 };
      }
      (IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW11_IPW33 == 0) ->
         (IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW20_IPW33 == 0) ;
      (IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW11_IPW33 == 0) ->
         (IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW20_IPW33 == 0) ;
      (IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW11_IPW33 <=
         IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW20_IPW33) ;
      (IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW11_IPW33 <=
         IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW20_IPW33) ;
   }
   constraint c_IPW08_counts_reasonable {
      IPW15_r.REG_IPW47_IPW08_CNTS[15: 0] dist { [32:63] :/ 25, 64 :/ 50, [65:192] :/ 25 };
      IPW15_r.REG_IPW47_IPW08_CNTS[31:16] dist { [32:63] :/ 25, 64 :/ 50, [65:192] :/ 25 };
      IPW15_r.REG_IPW13_INTF_THRESHOLDS[8:0] dist {
         0 :/ 1,
         [1 : ((`MAX_OUTSTANDING_IPW50_RDATA_LANES - 64) >> 1)] :/ 5,
         [((`MAX_OUTSTANDING_IPW50_RDATA_LANES - 64) >> 1) + 1 : ((`MAX_OUTSTANDING_IPW50_RDATA_LANES - 64) - 1)] :/ 11,
         [(`MAX_OUTSTANDING_IPW50_RDATA_LANES - 64) : 9'h1FF] :/ 3
      };
      IPW15_r.REG_IPW13_INTF_THRESHOLDS[24:16] dist {
         0 :/ 1,
         [1 : ((`IPW13_WDATA_MAX_CREDIT - 16) >> 1)] :/ 5,
         [((`IPW13_WDATA_MAX_CREDIT - 16) >> 1) + 1 : ((`IPW13_WDATA_MAX_CREDIT - 16))] :/ 11,
         [(`IPW13_WDATA_MAX_CREDIT - 16) + 1 : 9'h1FF] :/ 3
      };
   }
   // createRunargConstraints pattern (RUNARG_* != -1 -> fIPW59e value)
   constraint c_runargs {
      if (RUNARG_IPW19_IPW62 != -1) {
         IPW15_r.REG_IPW47_IPW19_IPW62 == RUNARG_IPW19_IPW62 ;
      }
      if (RUNARG_IPW08_CNTS != -1) {
         IPW15_r.REG_IPW47_IPW08_CNTS == RUNARG_IPW08_CNTS ;
      }
   }
endclass : XIPW47IPW55CtrlConfig

// ----- IPW02.sv:528 IPW47ClientCtrlConfig (constraints verbatim) -----
class XIPW47ClientCtrlConfig ;
   rand x_IPW04_regs_t IPW04_r ;
   IPW14_t IPW14_raw ;
   rand u4_t nIPW55s_pre ;
   u5_t first_IPW55_ra, num_IPW55s_ra ;
   u16_t IPW50_msk_ra ;
   bit [`NUM_TOTAL_IPW21-1:0] IPW21_iterator ;
   int RUNARG_REG_IPW47_IPW56_NUM_IPW55S = -1 ;

   function new();
   endfunction: new

   constraint c_num_IPW55s {
      nIPW55s_pre dist {
         0 :/  15, 1 :/  7, 2 :/  2, 3 :/  1
      };
      (IPW04_r.REG_IPW47_IPW56.NUM_IPW55S == (4'hf >> nIPW55s_pre));
   }
   constraint c_IPW56_cons {
      ( {1'b0,IPW04_r.REG_IPW47_IPW56.NUM_IPW55S} +
        {1'b0,IPW04_r.REG_IPW47_IPW56.FIRST_IPW55} ) <= (`NUM_IPW50 - 1) ;
      IPW04_r.REG_IPW47_IPW56.IPW47_IPW40_TYPE == 0 ;
   }
endclass : XIPW47ClientCtrlConfig

// ----- IPW02.sv:733 IPW47Config (StartCfgForDUT) -----
class XIPW47Config ;
   rand XIPW47IPW55CtrlConfig __IPW15s_cfg [`NUM_IPW50] ;
   rand XIPW47ClientCtrlConfig __IPW04_cfg ;
   rand XIPW50IPW32Registers __IPW50_sregs ;

   rand bit [15:0] total_IPW10_in_Gbits ;
   rand bit [15:0] IPW10_in_Gbits_perIPW50 ;
   rand bit [39:0] max_aIPW61ess, max_aIPW61ess_per_IPW50;
   rand bit [15:0] zero_sized_IPW26_dist [0:3] ;
   rand bit [3:0] full_access_IPW26_ids [0:3] ;
   rand bit [39:0] common_physical_aIPW61ess [0:3] ;
   rand bit [39:0] IPW43_for_cpa [0:3] ;

   bit bgrps_en ;
   u40_t init_only_aIPW61esses [] ;
   u40_t init_only_aIPW61esses_tos [] ;
   int ctr ;
   bit disable_post_rand_printout ;
   bit perf_diag ;
   bit disable_IPW60 ;
   bit [15:1] IPW26s_iterator ;
   bit [0:(`NUM_IPW60_IPW29S-1)] IPW29s_iterator ;
   bit [`NUM_TOTAL_IPW06-1:0] IPW06_iterator ;
   bit [`NUM_TOTAL_IPW21-1:0] IPW21_iterator ;

   function new(bit dprp = 0);
      __IPW50_sregs = new();
      __IPW04_cfg = new();
      foreach (__IPW15s_cfg[i]) begin
         __IPW15s_cfg[i] = new();
      end
      disable_post_rand_printout = dprp ;
      perf_diag = $test$plusargs("IPW47_perf_test");
      disable_IPW60 = $test$plusargs("disable_IPW60") | perf_diag ;
   endfunction: new

   function void pre_randomize();
      if (perf_diag | disable_IPW60) begin
         c_ensure_at_leIPW60_one_full_access_IPW26_for_all_aIPW61ess_widths.constraint_mode(0);
         c_zero_sized_IPW26s_dist.constraint_mode(0);
         c_IPW60_seg_limits_reloc.constraint_mode(0);
      end
   endfunction: pre_randomize

   constraint c_disabled_IPW60 {
      if (disable_IPW60) {
         foreach (IPW29s_iterator[d]) {
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[(d << 4) + 0] == 0 ;
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RIGHTS[(d << 4) + 0] == 4'b1011 ;
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(d << 4) + 15] == max_aIPW61ess[39:16] ;
            foreach (IPW26s_iterator[i]) {
               __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[(d << 4) + i] == 0 ;
               __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RIGHTS[(d << 4) + i] == 4'b1011 ;
               __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(d << 4) + i - 1] == 24'h0 ;
            }
         }
      }
   }
   constraint c_avoid_64B_bfs_in_2IPW55_IPW56 {
      (__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S == 1) ->
         (__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.IPW47_IPW07_SIZE == 1);
   }
   constraint c_same_IPW50_params_across_all_bIPW52s {
      __IPW04_cfg.IPW04_r.REG_IPW47_IPW56.IPW09 == __IPW50_sregs.IPW09_ds ;
      __IPW04_cfg.IPW04_r.REG_IPW47_IPW56.IPW47_IPW07_SIZE == __IPW50_sregs.IPW07_size_ds ;
   }
   constraint c_IPW63_lat_thresh_config {
      foreach (IPW21_iterator[i]) {
         __IPW04_cfg.IPW04_r.REG_IPW47_IPW63_IPW25_THRESHOLDS[int'(i << 5) +: 16] dist {
            0 :/ 2, [16:1023] :/ 2, [1024:4095] :/ 5, [4096:65535] :/ 1
         };
      }
   }
   constraint c_perf_config {
      if (perf_diag) {
         (__IPW50_sregs.IPW09_ra == -1) -> (__IPW50_sregs.IPW09_ds == 1) ;
         if (__IPW04_cfg.RUNARG_REG_IPW47_IPW56_NUM_IPW55S == -1) {
            (__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S == (`NUM_IPW50-1)) ;
         }
      }
   }

   constraint c_solving_order {
      solve __IPW50_sregs.IPW10_ds before total_IPW10_in_Gbits ;
      solve __IPW50_sregs.IPW18_en_ds before total_IPW10_in_Gbits ;
      solve __IPW04_cfg.IPW04_r before total_IPW10_in_Gbits ;
      solve total_IPW10_in_Gbits before max_aIPW61ess ;
      solve IPW10_in_Gbits_perIPW50 before total_IPW10_in_Gbits ;
      // §18.5: ordering directives are constraint block items. This
      // fixture has two controllers, so the second one is at index 1.
      solve full_access_IPW26_ids before __IPW15s_cfg[0].IPW15_r ;
      solve __IPW04_cfg.IPW04_r before __IPW15s_cfg[0].IPW15_r ;
      solve __IPW15s_cfg[0].IPW15_r before __IPW15s_cfg[1].IPW15_r ;
   }
   constraint c_compute_total_IPW10 {
      IPW10_in_Gbits_perIPW50 == (__IPW50_sregs.IPW10_per_IPW55_per_die_per_x16[__IPW50_sregs.IPW10_ds]
                                      << __IPW50_sregs.IPW18_en_ds);
      total_IPW10_in_Gbits == IPW10_in_Gbits_perIPW50 * (__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S + 1);
      max_aIPW61ess == ( (40'h800_0000 * total_IPW10_in_Gbits) - 1 );
      max_aIPW61ess_per_IPW50 == ( (40'h800_0000 * IPW10_in_Gbits_perIPW50) - 1 );
   }
   constraint c_ensure_at_leIPW60_one_full_access_IPW26_for_all_aIPW61ess_widths {
      foreach (full_access_IPW26_ids[i]) {
         __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RIGHTS[ (i << 4) + full_access_IPW26_ids[i] ] == 4'b1011 ;
         __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + full_access_IPW26_ids[i] ][23:16] == 8'h0 ;
         zero_sized_IPW26_dist[i][full_access_IPW26_ids[i]] == 1'b0;
      }
   }
   constraint c_zero_sized_IPW26s_dist {
      foreach (zero_sized_IPW26_dist[i]) {
         zero_sized_IPW26_dist[i] dist { 0 :/ 9, [1:16'hFFFF] :/ 1 };
         $countones(zero_sized_IPW26_dist[i]) <= 15 ;
      }
   }
   constraint c_IPW60_seg_limits_reloc {
      foreach (IPW29s_iterator[i]) {
         zero_sized_IPW26_dist[i][0]  -> (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + 0 ] == 24'h0);
         !zero_sized_IPW26_dist[i][0] -> (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + 0 ] != 24'h0);
         foreach (IPW26s_iterator[j]) {
            if (zero_sized_IPW26_dist[i][j]) {
               (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j ] ==
                  __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j - 1 ]) ;
            } else {
               (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j ] >
                  __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j - 1 ]) ;
            }
         }
      }
      foreach (IPW29s_iterator[i]) {
         __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) ] < max_aIPW61ess[39:16] ;
         __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) ] < max_aIPW61ess[39:16] ;
         __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) ][4:0] == 0;
         (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) ] +
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) ]) < max_aIPW61ess[39:16] ;
         foreach (IPW26s_iterator[j]) {
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j ] < max_aIPW61ess[39:16] ;
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) + j ] < max_aIPW61ess[39:16] ;
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) + j ][4:0] == 0;
            (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + j ] +
               __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) + j ]) < max_aIPW61ess[39:16] ;
         }
      }
   }
   constraint c_common_physical_aIPW61ess {
      foreach (IPW29s_iterator[i]) {
         if (full_access_IPW26_ids[i] == 0) {
            IPW43_for_cpa[i][39:16] inside { [
               0 : __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + full_access_IPW26_ids[i] ]
            ]};
         } else {
            IPW43_for_cpa[i][39:16] inside { [
               (__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + full_access_IPW26_ids[i] - 1] + 1)
               : __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[ (i << 4) + full_access_IPW26_ids[i] ]
            ]};
         }
         IPW43_for_cpa[i][15:0] == 0 ;
         common_physical_aIPW61ess[i][15:0] == 0 ;
         common_physical_aIPW61ess[i][39:16] == IPW43_for_cpa[i][39:16] +
            __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[ (i << 4) + full_access_IPW26_ids[i] ] ;
         common_physical_aIPW61ess[i][39:16] < max_aIPW61ess[39:16] ;
      }
      common_physical_aIPW61ess[0] == common_physical_aIPW61ess[1] ;
      common_physical_aIPW61ess[1] == common_physical_aIPW61ess[2] ;
      common_physical_aIPW61ess[2] == common_physical_aIPW61ess[3] ;
      common_physical_aIPW61ess[3] == common_physical_aIPW61ess[0] ;
   }
   constraint c_cfg_dep_chain {
      __IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW56_DUP == __IPW04_cfg.IPW04_r.REG_IPW47_IPW56 ;
   }
endclass : XIPW47Config

"#;

const REPRO: &str = r#"
// ============================================================================
// Test: S02 pattern (first randomize) + S03 pattern (runarg tweaks + re-rand)
// ============================================================================
module mwe1_class_randomize ;
   int failures = 0 ;

   task automatic chk(bit ok, string msg);
      if (!ok) begin
         failures++ ;
         $display("FAIL : %s", msg) ;
      end
   endtask

   initial begin
      // ---- S02 pattern: StartCfgForDUT.randomize() ----
      XIPW47Config StartCfgForDUT = new ;
      chk(StartCfgForDUT.randomize() == 1, "S02: randomize returned 0") ;
      chk(StartCfgForDUT.total_IPW10_in_Gbits > 0, "S02: total_IPW10_in_Gbits == 0") ;
      chk(StartCfgForDUT.max_aIPW61ess == ((40'h800_0000 * StartCfgForDUT.total_IPW10_in_Gbits) - 1),
          "S02: max_aIPW61ess mismatch") ;
      chk(StartCfgForDUT.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW19_IPW62 > 0, "S02: IPW19_IPW62 == 0") ;
      chk(StartCfgForDUT.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.IPW47_IPW40_TYPE == 0, "S02: IPW40_TYPE != 0") ;

      // ---- S03 pattern: tweak runargs between calls, re-randomize ----
      void'(StartCfgForDUT.randomize()) ;
      StartCfgForDUT.__IPW50_sregs.IPW09_ra = 1 ;
      StartCfgForDUT.__IPW04_cfg.RUNARG_REG_IPW47_IPW56_NUM_IPW55S = 1 ;
      chk(StartCfgForDUT.randomize() == 1, "S03: re-randomize returned 0") ;
      chk(StartCfgForDUT.__IPW50_sregs.IPW09_ds == 1, "S03: IPW09_ds != runarg 1") ;
      chk(StartCfgForDUT.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S == 1,
          "S03: NUM_IPW55S != 1") ;

      if (failures == 0) $display("TEST_PASS");
      else begin $display("TEST_FAIL count=%0d", failures); $fatal(1); end
      $finish ;
   end
endmodule

"#;

const CHECK: &str = r#"
// Procedural restatement of every constraint of the solve (all blocks on,
// cov_tweaks/perf_diag/disable_IPW60 off, runargs as set).
function automatic int check_cfg(XIPW47Config c);
   int bad = 0;
   XIPW50IPW32Registers r = c.__IPW50_sregs;
   XIPW47ClientCtrlConfig cl = c.__IPW04_cfg;
   bit [39:0] mx;
   // sregs
   if (r.IPW09_ds > 1) bad++;            // dist {0,1}
   if (r.IPW10_ds > 8) bad++;
   if (r.IPW09_ra != -1 && r.IPW09_ds != r.IPW09_ra) bad++;
   // client
   if (!(cl.nIPW55s_pre inside {[0:3]})) bad++;
   if (cl.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S != (4'hf >> cl.nIPW55s_pre)) bad++;
   if (({1'b0,cl.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S} + {1'b0,cl.IPW04_r.REG_IPW47_IPW56.FIRST_IPW55}) > 1) bad++;
   if (cl.IPW04_r.REG_IPW47_IPW56.IPW47_IPW40_TYPE != 0) bad++;
   if (cl.RUNARG_REG_IPW47_IPW56_NUM_IPW55S != -1 && 0) bad++;
   for (int i = 0; i < 17; i++) begin
      bit [15:0] t = cl.IPW04_r.REG_IPW47_IPW63_IPW25_THRESHOLDS[i*32 +: 16];
      if (!(t == 0 || (t >= 16))) bad++;   // dist items 0, [16:65535]
   end
   // ctrl configs
   foreach (c.__IPW15s_cfg[k]) begin
      XIPW47IPW55CtrlConfig q = c.__IPW15s_cfg[k];
      if (!(q.IPW15_r.REG_IPW47_IPW19_IPW62 > 0)) bad++;
      if (!(q.IPW15_r.REG_IPW47_IPW19_IPW62 inside {[1:60000]})) bad++;
      if ((q.IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW11_IPW33 == 0) && (q.IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW20_IPW33 != 0)) bad++;
      if ((q.IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW11_IPW33 == 0) && (q.IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW20_IPW33 != 0)) bad++;
      if (q.IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW11_IPW33 > q.IPW15_r.REG_IPW47_POST_RD_IPW12_IPW33.IPW20_IPW33) bad++;
      if (q.IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW11_IPW33 > q.IPW15_r.REG_IPW47_POST_WR_IPW12_IPW33.IPW20_IPW33) bad++;
      if (!(q.IPW15_r.REG_IPW47_IPW08_CNTS[15:0] inside {[32:192]})) bad++;
      if (!(q.IPW15_r.REG_IPW47_IPW08_CNTS[31:16] inside {[32:192]})) bad++;
   end
   // parent
   if (c.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S == 1 && c.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.IPW47_IPW07_SIZE != 1) bad++;
   if (cl.IPW04_r.REG_IPW47_IPW56.IPW09 != r.IPW09_ds) bad++;
   if (cl.IPW04_r.REG_IPW47_IPW56.IPW47_IPW07_SIZE != r.IPW07_size_ds) bad++;
   if (c.IPW10_in_Gbits_perIPW50 != (r.IPW10_per_IPW55_per_die_per_x16[r.IPW10_ds] << r.IPW18_en_ds)) bad++;
   if (c.total_IPW10_in_Gbits != c.IPW10_in_Gbits_perIPW50 * (cl.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S + 1)) bad++;
   if (c.max_aIPW61ess != ((40'h800_0000 * c.total_IPW10_in_Gbits) - 1)) bad++;
   if (c.max_aIPW61ess_per_IPW50 != ((40'h800_0000 * c.IPW10_in_Gbits_perIPW50) - 1)) bad++;
   mx = c.max_aIPW61ess;
   for (int i = 0; i < 4; i++) begin
      bit [3:0] id = c.full_access_IPW26_ids[i];
      if (c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RIGHTS[(i << 4) + id] != 4'b1011) bad++;
      if (c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + id][23:16] != 8'h0) bad++;
      if (c.zero_sized_IPW26_dist[i][id] != 1'b0) bad++;
      if ($countones(c.zero_sized_IPW26_dist[i]) > 15) bad++;
      if (c.zero_sized_IPW26_dist[i][0]  && c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4)] != 24'h0) bad++;
      if (!c.zero_sized_IPW26_dist[i][0] && c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4)] == 24'h0) bad++;
      for (int j = 1; j < 16; j++) begin
         bit [23:0] a = c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + j];
         bit [23:0] b = c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + j - 1];
         if (c.zero_sized_IPW26_dist[i][j]) begin if (a != b) bad++; end
         else if (!(a > b)) bad++;
      end
      for (int j = 0; j < 16; j++) begin
         bit [23:0] l = c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + j];
         bit [23:0] o = c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[(i << 4) + j];
         bit [23:0] s24 = l + o;
         if (!(l < mx[39:16])) bad++;
         if (!(o < mx[39:16])) bad++;
         if (o[4:0] != 0) bad++;
         if (!(s24 < mx[39:16])) bad++;
      end
      if (id == 0) begin
         if (!(c.IPW43_for_cpa[i][39:16] <= c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + id])) bad++;
      end else begin
         if (!(c.IPW43_for_cpa[i][39:16] >= c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + id - 1] + 24'd1 &&
               c.IPW43_for_cpa[i][39:16] <= c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_LIMIT[(i << 4) + id])) bad++;
      end
      if (c.IPW43_for_cpa[i][15:0] != 0) bad++;
      if (c.common_physical_aIPW61ess[i][15:0] != 0) bad++;
      begin
        bit [23:0] cs = c.IPW43_for_cpa[i][39:16] + c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW60_SEG_RELOC[(i << 4) + id];
        if (c.common_physical_aIPW61ess[i][39:16] != cs) bad++;
      end
      if (!(c.common_physical_aIPW61ess[i][39:16] < mx[39:16])) bad++;
   end
   for (int i = 1; i < 4; i++) if (c.common_physical_aIPW61ess[i] != c.common_physical_aIPW61ess[0]) bad++;
   if (c.__IPW15s_cfg[0].IPW15_r.REG_IPW47_IPW56_DUP != cl.IPW04_r.REG_IPW47_IPW56) bad++;
   return bad;
endfunction

module chk;
   initial begin
      XIPW47Config c = new;
      int ok, bad;
      ok = c.randomize(); bad = check_cfg(c);
      $display("s02 ok=%0d bad=%0d num=%0d", ok, bad, c.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S);
      ok = c.randomize(); bad = check_cfg(c);
      $display("s02b ok=%0d bad=%0d", ok, bad);
      c.__IPW50_sregs.IPW09_ra = 1;
      c.__IPW04_cfg.RUNARG_REG_IPW47_IPW56_NUM_IPW55S = 1;
      ok = c.randomize(); bad = check_cfg(c);
      $display("s03 ok=%0d bad=%0d ds=%0d num=%0d", ok, bad, c.__IPW50_sregs.IPW09_ds, c.__IPW04_cfg.IPW04_r.REG_IPW47_IPW56.NUM_IPW55S);
   end
endmodule
"#;
