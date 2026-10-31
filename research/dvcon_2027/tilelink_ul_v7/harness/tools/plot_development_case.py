"""Standalone figure from actual development pilot pin records, with audit data."""
import hashlib,json,sys
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'tools'))
from check_pins import parse
exp=ROOT/'results/pilot_case_02_D_001/experiment_01';out=ROOT/'paper/figures';out.mkdir(exist_ok=True)
paths={name:exp/name/'stdout.log' for name in ('baseline','experiment','unrelated','idempotent')};paths['repair']=ROOT/'results/case02_D_patch_controls_001/experiment/stdout.log';samples={name:parse(p.read_text())[0] for name,p in paths.items()};rows={name:[s for s in rr if 19<=s['cycle']<=34] for name,rr in samples.items()}
assert samples['baseline']==samples['unrelated']==samples['idempotent']
fig,axes=plt.subplots(4,1,figsize=(8.2,6.5),sharex=True,layout='constrained')
fields=[('dv','D valid'),('dr','D ready'),('dsrc','D source'),('asrc','A source')]
for ax,(field,label) in zip(axes,fields):
 for name,color,style in [('baseline','#243a73','-'),('experiment','#bc392c','--'),('repair','#2a8043','-.')]:
  ax.step([s['cycle'] for s in rows[name]],[s[field] for s in rows[name]],where='post',label='baseline + both negative controls' if name=='baseline' else ('agent-selected backpressure' if name=='experiment' else 'real patch + same backpressure (engineering replay)'),color=color,ls=style,lw=1.7)
 ax.axvspan(23.5,31.5,color='#bc392c',alpha=.07);ax.axvline(24,color='.35',lw=.8,ls=':');ax.grid(alpha=.18);ax.set_ylabel(label)
axes[0].axvspan(19,25,color='#2384b6',alpha=.10,label='native pretrigger window (235–<295 ns)');axes[0].axvline(25,color='#2384b6',ls=':',lw=1);axes[0].legend(loc='upper right',fontsize=7);axes[0].set_ylim(-.2,1.9);axes[1].set_ylim(-.2,1.2)
axes[-1].set_xlabel('PIN cycle (posedge samples; lines connect samples)');axes[-1].set_xticks(range(19,35));axes[-1].set_xlim(19,34)
fig.suptitle('Development-only case: premature response retirement\nFirst independent stall violation at cycle 24',fontsize=12)
fig.savefig(out/'development_case_timeline.svg');fig.savefig(out/'development_case_timeline.pdf');fig.savefig(out/'development_case_timeline.png',dpi=140);plt.close(fig)
receipt={'status':'DEVELOPMENT_ONLY','input_sha256':{name:hashlib.sha256(p.read_bytes()).hexdigest() for name,p in paths.items()},'samples':rows,'negative_controls_identical':True,'force_negedge_counter':23,'release_negedge_counter':31,'source_hidden_seal_sha256':'504b1a40cee638084894ee6c15aba2f2e000e1d23361fd56597fb93636677d97','limitations':'Stock pilot tool surface unqualified; not primary efficacy evidence. Shading is between sampled posedges using observed CONTROL markers.'}
(out/'development_case_timeline_data.json').write_text(json.dumps(receipt,indent=2)+'\n')
print('Wrote figure and actual source-data receipt')
