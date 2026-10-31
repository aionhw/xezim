#!/usr/bin/env python3
"""New temporal PILOT ONLY constructs; never a primary holdout generator."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'private/temporal_pilots_001';out.mkdir(exist_ok=False)
components=r'''`timescale 1ns/1ps
module tlul_traffic_adapter(input wire clk,rst_n,av,ar,dv,dr,
input wire [2:0] op,input wire [3:0] src,mask,input wire [15:0] addr,
input wire [31:0] data,output wire [15:0] delivered_addr,output wire [31:0] delivered_data);
reg [31:0] previous_write;
reg [15:0] previous_address;
reg stalled_last;
integer accepted_count;
always @(posedge clk) begin
 if(!rst_n) begin previous_write<=0;previous_address<=0;stalled_last<=0;accepted_count<=0;end
 else begin
  stalled_last<=dv&&!dr;
  if(av&&ar) begin accepted_count<=accepted_count+1;
   if(op!=4) begin previous_write<=data;previous_address<=addr;end
  end
 end
end
assign delivered_data=data; // TRAFFIC_DATA
assign delivered_addr=addr; // TRAFFIC_ADDRESS
endmodule
module tlul_event_monitor(input wire clk,rst_n,av,ar,dv,dr,
input wire [3:0] asrc,dsrc,output wire alarm);
reg monitor_pending[0:15];integer generation[0:15];integer admitted_generation[0:15];
integer serial;integer i;
always @(posedge clk) begin
 if(!rst_n) begin serial<=0;for(i=0;i<16;i=i+1) begin monitor_pending[i]<=0;generation[i]<=0;admitted_generation[i]<=0;end end
 else begin
  if(av&&ar) begin monitor_pending[asrc]<=1;admitted_generation[asrc]<=generation[asrc];serial<=serial+1;end
  if(dv&&dr) begin monitor_pending[dsrc]<=0;generation[dsrc]<=generation[dsrc]+1;end // MONITOR_RETIRE
 end
end
assign alarm=rst_n&&((av&&ar&&monitor_pending[asrc])||(dv&&dr&&(!monitor_pending[dsrc]||admitted_generation[dsrc]!=generation[dsrc]))); // MONITOR_GENERATION
endmodule
module tlul_shadow_scoreboard(input wire clk,rst_n,av,ar,dv,dr,
input wire [2:0] aop,dop,input wire [3:0] asrc,dsrc,mask,
input wire [15:0] addr,input wire [31:0] adata,ddata,output wire alarm);
reg [31:0] model_memory[0:255];reg [31:0] expected[0:15];reg [7:0] read_address[0:15];reg [31:0] scratch;integer i;
always @(posedge clk) begin
 if(!rst_n) begin for(i=0;i<256;i=i+1) model_memory[i]<=0;for(i=0;i<16;i=i+1) begin expected[i]<=0;read_address[i]<=0;end end
 else if(av&&ar) begin
  if(aop==4) begin expected[asrc]<=model_memory[addr>>2];read_address[asrc]<=addr>>2;end
  else begin scratch=model_memory[addr>>2];
   if(mask[0])scratch[7:0]=adata[7:0];if(mask[1])scratch[15:8]=adata[15:8];if(mask[2])scratch[23:16]=adata[23:16];if(mask[3])scratch[31:24]=adata[31:24];model_memory[addr>>2]<=scratch;
  end
 end
end
assign alarm=rst_n&&dv&&dr&&dop==1&&(ddata!==expected[dsrc]); // SCOREBOARD_SNAPSHOT
endmodule
'''
(out/'clean_components.sv').write_text(components)
tb=(ROOT/'rtl/tb_tlul.sv').read_text();decl='''
wire [15:0] delivered_a_address;wire [31:0] delivered_a_data;
wire monitor_alarm,scoreboard_alarm;
tlul_traffic_adapter traffic(.clk(clk),.rst_n(rst_n),.av(a_valid),.ar(a_ready),.dv(d_valid),.dr(d_ready),.op(a_opcode),.src(a_source),.mask(a_mask),.addr(a_address),.data(a_data),.delivered_addr(delivered_a_address),.delivered_data(delivered_a_data));
tlul_event_monitor monitor(.clk(clk),.rst_n(rst_n),.av(a_valid),.ar(a_ready),.dv(d_valid),.dr(d_ready),.asrc(a_source),.dsrc(d_source),.alarm(monitor_alarm));
tlul_shadow_scoreboard scoreboard(.clk(clk),.rst_n(rst_n),.av(a_valid),.ar(a_ready),.dv(d_valid),.dr(d_ready),.aop(a_opcode),.dop(d_opcode),.asrc(a_source),.dsrc(d_source),.mask(a_mask),.addr(a_address),.adata(a_data),.ddata(d_data),.alarm(scoreboard_alarm));
'''
tb=tb.replace('  tlul_mem_endpoint dut (',decl+'\n  tlul_mem_endpoint dut (');tb=tb.replace('.a_size(a_size), .a_source(a_source), .a_address(a_address),','.a_size(a_size), .a_source(a_source), .a_address(delivered_a_address),').replace('.a_mask(a_mask), .a_data(a_data),','.a_mask(a_mask), .a_data(delivered_a_data),');tb=tb.replace('      if (a_valid && a_ready) begin','      $display("MON|cycle=%0d|source_alarm=%0d|data_alarm=%0d",cycle,monitor_alarm,scoreboard_alarm);\n      if (a_valid && a_ready) begin',1);(out/'tb_temporal.sv').write_text(tb)
endpoint=(ROOT/'rtl/tlul_mem_endpoint.sv').read_text();catalog={}
def add(key,role,original,text,mechanism):
 assert original!=text
 p=out/(key+'.sv');p.write_text(text);catalog[key]={'role':role,'source':str(p.resolve()),'tb':str((out/'tb_temporal.sv').resolve()),'mechanism_private':mechanism,'source_sha256':hashlib.sha256(text.encode()).hexdigest(),'eligibility':'PILOT_ONLY_PENDING_ADMISSION','patch_target':'rtl/tlul_mem_endpoint.sv' if role=='endpoint' else 'rtl/tlul_components.sv','extra_sources':[str((out/'clean_components.sv').resolve())] if role=='endpoint' else [str((ROOT/'rtl/tlul_mem_endpoint.sv').resolve())]}
add('temporal_01','traffic',components,components.replace('assign delivered_data=data;','assign delivered_data=(stalled_last&&op==1&&accepted_count>=16)?previous_write:data;'),'A partial write after a D stall reuses the previous accepted write payload rather than the current payload.')
add('temporal_02','traffic',components,components.replace('assign delivered_addr=addr;','assign delivered_addr=(accepted_count>=16&&op==1&&previous_address[3:2]==addr[3:2])?previous_address:addr;'),'A bank-tag hit after prior traffic routes a later partial write using an old address descriptor.')
add('temporal_03','monitor',components,components.replace('if(dv&&dr) begin monitor_pending[dsrc]<=0;','if(dv) begin monitor_pending[dsrc]<=0;'),'The monitor retires an observed response on valid before the stalled transfer is accepted.')
add('temporal_04','monitor',components,components.replace('admitted_generation[asrc]<=generation[asrc];','admitted_generation[asrc]<=serial;'),'Monitor correlates a source-local completion epoch with the global request serial, producing false alarms under concurrent sources/reuse.')
add('temporal_05','scoreboard',components,components.replace('ddata!==expected[dsrc]','ddata!==model_memory[read_address[dsrc]]'),'The scoreboard compares a delayed read response against memory after intervening writes, rather than its accepted-request snapshot.')
# Endpoint temporal cases retain the clean traffic adapter/monitor/scoreboard.
a='  reg [31:0] write_word;';new=a+'\n  reg [31:0] retired_read; reg stalled_before;\n  always @(posedge clk) begin\n    if(!rst_n) begin retired_read<=0;stalled_before<=0;end\n    else begin stalled_before<=d_valid&&!d_ready; if(d_valid&&d_ready&&d_opcode==1) retired_read<=d_data;end\n  end'
e=endpoint.replace(a,new).replace('response_data[a_source] <= memory[word_index];','response_data[a_source] <= (stalled_before&&cycle_count>40)?retired_read:memory[word_index];');add('temporal_06','endpoint',endpoint,e,'After a D stall, a new read captures the last retired read payload instead of the addressed memory snapshot.')
e=endpoint.replace(a,a+'\n  reg [31:0] bank_snapshot[0:3];\n  always @(posedge clk) begin\n    if(!rst_n) begin for(integer b=0;b<4;b=b+1) bank_snapshot[b]<=0;end\n    else if(a_valid&&a_ready&&a_opcode==4) bank_snapshot[a_address[3:2]]<=memory[a_address>>2];\n  end').replace('write_word = memory[word_index];','write_word = (cycle_count>40&&a_opcode==1&&d_valid&&!d_ready)?bank_snapshot[a_address[3:2]]:memory[word_index];');add('temporal_07','endpoint',endpoint,e,'A masked write during D backpressure merges into a bank read snapshot, losing intervening byte updates.')
e=endpoint.replace('      if (d_valid && d_ready) begin','      if(d_valid&&!d_ready&&cycle_count>40&&pending[(d_source+1)%SOURCES]&&response_opcode[(d_source+1)%SOURCES]==1)\n        response_data[(d_source+1)%SOURCES] <= response_data[d_source];\n      if (d_valid && d_ready) begin');add('temporal_08','endpoint',endpoint,e,'During a held D response, a neighboring pending descriptor is refreshed from the selected response payload, corrupting a later read.')
(out/'catalog_private.json').write_text(json.dumps(catalog,indent=2)+'\n');print('Created eight PILOT ONLY candidates; all require admission.')
