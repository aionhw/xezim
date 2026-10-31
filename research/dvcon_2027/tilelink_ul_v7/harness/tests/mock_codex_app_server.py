#!/usr/bin/env python3
"""OFFLINE TEST FAKE: it never contacts a model. NEVER count as DVCon trial."""
import json, os, sys
from pathlib import Path


def send(value):
    print(json.dumps(value), flush=True)


def read():
    s=sys.stdin.readline()
    return json.loads(s) if s else None


def main():
    stage=0
    while True:
        msg=read()
        if msg is None:break
        if 'id' not in msg:continue
        method=msg.get('method')
        id_=msg['id']
        if method=='initialize':
            if not msg['params']['capabilities']['experimentalApi']:raise AssertionError('missing experimental opt-in')
            send({'id':id_,'result':{'userAgent':'mock; not a real service'}})
        elif method=='account/read':
            send({'id':id_,'result':{'account':{'type':'chatgpt','planType':'pro','email':'HIDDEN_EMAIL'},'requiresOpenaiAuth':True}})
        elif method=='model/list':
            send({'id':id_,'result':{'data':[{'model':'gpt-6.1-sol','supportedReasoningEfforts':[{'reasoningEffort':'medium'}]}]}})
        elif method=='thread/start':
            assert msg['params']['model']=='gpt-6.1-sol'
            assert msg['params']['config']['default_permissions']=='dvcon_case'
            assert msg['params']['config']['features.memories'] is False
            assert msg['params']['config']['features.shell_tool'] is False
            assert msg['params']['config']['features.multi_agent'] is False
            assert len(msg['params']['dynamicTools'])>=1
            send({'id':id_,'result':{'thread':{'id':'MOCK-THREAD-001'},'instructionSources':[],'model':msg['params']['model'],'activePermissionProfile':{'id':'dvcon_case'},'runtimeWorkspaceRoots':[msg['params']['cwd']]}})
        elif method=='turn/start':
            stage+=1
            assert msg['params']['effort']=='medium'
            assert msg['params']['permissions']=='dvcon_case'
            input_text=msg['params']['input'][0]['text']
            if stage==1:
                assert 'SECRET_SOURCE_MARKER' not in input_text, 'source leaked before seal'
            elif stage==2:
                assert 'SECRET_SOURCE_MARKER' in input_text, 'no Stage II source disclosure'
                assert Path(os.environ['MOCK_TRIAL_OUT'],'stage1_sealed.json').exists(), 'source revealed before Stage I seal'
            turn='MOCK-TURN-'+str(stage)
            send({'id':id_,'result':{'turn':{'id':turn,'status':'inProgress'}}})
            if stage==1:
                send({'id':900,'method':'item/tool/call',
                    'params':{'threadId':'MOCK-THREAD-001','turnId':turn,
                              'tool':'read_evidence','arguments':{'kind':'pin','start':0,'count':2}}})
                reply=read()
                assert reply['id']==900,reply
                result=json.loads(reply['result']['contentItems'][0]['text'])
                assert result['citation_id']=='tlul_ev_0001'
                output={'assessment':'defect','component':'endpoint','mechanism':'test assertion',
                     'two_alternative_hypotheses':['A','B'],
                     'supporting_issued_evidence':['invented_id' if os.environ.get('MOCK_BAD_CITATION') else result['citation_id']],
                     'predicted_discriminating_result':'mock only','confidence':0.4}
            else:
                output={'path':'rtl/tlul_mem_endpoint.sv','old':'SECRET_SOURCE_MARKER',
                        'new':'SECRET_SOURCE_MARKER','explanation':'mock only'}
            send({'method':'item/completed','params':{'item':{'type':'agentMessage','text':json.dumps(output)}}})
            send({'method':'thread/tokenUsage/updated','params':{'tokenUsage':{'inputTokens':1234,'cachedInputTokens':111}}})
            send({'method':'turn/completed','params':{'turn':{'id':turn,'status':'completed'}}})
        else:raise AssertionError('unknown method: '+str(method))

if __name__=='__main__':main()
