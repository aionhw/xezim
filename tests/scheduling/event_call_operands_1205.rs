//! IEEE 1800-2023 §9.4.2: an explicit event tracks call arguments and
//! wakes only when the value of its expression changes.
#[test]
fn call_events_track_arguments_and_results() {
    let s = r#"
module top;
 timeunit 1ns; timeprecision 1ps;
 real r=1.25;
 int a=0,b=0,c=0,d=0,e=0,f=0,g=0,h=0;
 int v[2]='{0,0}; int idx=0;
 function real project(real x); return x; endfunction
 function int stable(real x); return 7; endfunction
 function int identity(int x); return x; endfunction
 initial begin #0.5; forever @(project(r)) a++; end
 initial begin #0.5; forever @($rtoi(r*4)) b++; end
 initial begin #0.5; forever @(stable(r)) c++; end
 initial begin #0.5; forever @((project(r))) d++; end
 initial begin #0.5; forever @(identity(v[idx])) e++; end
 initial begin #0.5; forever @(posedge identity(v[idx])) f++; end
 initial begin #0.5; forever @(project(.x(r))) g++; end
 always @(project(r)) if ($time > 0) h++;
 initial begin
  #1 r=2.5; v[1]=1;
  #1 r=2.6; idx=1;
  #1 v[0]=2;
  #1 v[1]=0;
  #1 $display("counts=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",a,b,c,d,e,f,g,h); $finish;
 end
endmodule
"#;
    let sim = xezim::simulate(s, 100_000).expect("simulate");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("counts=2,1,0,2,2,1,2,2"), "{out}");
}
