"""Publication figure for matched real recorder cost measurements."""
import json,re
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
ROOT=Path(__file__).resolve().parents[1];r=json.loads((ROOT/'results/matched_recorders_001/receipt.json').read_text());out=ROOT/'paper/figures';out.mkdir(exist_ok=True)
fig,axes=plt.subplots(1,3,figsize=(9,3),layout='constrained');modes=['none','vcd','value','native']
for i,mode in enumerate(modes):
 rows=[x for x in r['runs'] if x['mode']==mode]
 vals=[[x['wall_seconds']*1000 for x in rows],[x['peak_rss_kib']/1024 for x in rows],[x['raw_recorder_bytes']/1024 for x in rows]]
 for ax,values in zip(axes,vals):
  ax.scatter([i-.08,i,i+.08],values,color=['#777777','#3d6b9e','#56874d','#af5739'][i],s=24)
for ax,title,ylabel in zip(axes,['Invocation wall','Peak RSS','Raw recorder output'],['ms','MiB','KiB']):
 ax.set_xticks(range(4),['none','VCD','value','native'],rotation=25);ax.set_title(title,fontsize=10);ax.set_ylabel(ylabel);ax.grid(axis='y',alpha=.2)
fig.suptitle('Matched sampled-field coverage; three rotating serial repeats',fontsize=11)
fig.savefig(out/'matched_recorder_costs.svg');fig.savefig(out/'matched_recorder_costs.pdf');plt.close(fig)
# Only sanitized measured fields; no source keys or transcripts.
(out/'matched_recorder_costs_data.json').write_text(json.dumps({'status':r['status'],'scope':r['scope'],'binary_sha256':r['binary_sha256'],'runs':[{k:x[k] for k in ('repetition','mode','wall_seconds','peak_rss_kib','raw_recorder_bytes','phase_records','samples','normalized_missing_fields')} for x in r['runs']]},indent=2)+'\n')
