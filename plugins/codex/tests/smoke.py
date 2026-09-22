"""Exercise the packaged plugin in the actual gateway against local Codex/OAuth mocks."""
import base64
import concurrent.futures
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

ROOT=Path(__file__).resolve().parents[3]
PLUGIN=Path(__file__).resolve().parents[1]
report=[]
class Upstream(BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    lock=threading.Lock()
    refreshes=0
    serial=0
    requests=[]
    def log_message(self,*args):pass
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get('content-length',0)))
        data=json.loads(raw) if self.headers.get('content-type','').startswith('application/json') else raw.decode()
        status=200;headers={}
        if self.path.endswith('/usercode'):
            value={'device_auth_id':'device-id','user_code':'TEST','expires_in':900,'interval':1}
        elif self.path.endswith('/deviceauth/token'):
            value={'authorization_code':'fixture-code','code_verifier':'fixture-verifier'}
        elif self.path=='/oauth/token':
            if 'grant_type=refresh_token' in data:
                with self.lock:Upstream.refreshes+=1
                time.sleep(.1)
                value={'access_token':'fresh-access','expires_in':3600}
            else:
                claims=base64.urlsafe_b64encode(json.dumps({'https://api.openai.com/auth':{'chatgpt_account_id':'device-account'}}).encode()).decode().rstrip('=')
                value={'access_token':'device-access','refresh_token':'device-refresh','expires_in':3600,'id_token':'x.'+claims+'.x'}
        else:
            with self.lock:
                Upstream.serial+=1;serial=Upstream.serial
                Upstream.requests.append({'path':self.path,'headers':dict(self.headers),'body':data})
            account=self.headers.get('chatgpt-account-id')
            text=json.dumps(data.get('input'))
            if 'http-fail' in text:
                status=400;value={'error':{'message':'bad fixture','type':'invalid_request_error'}}
            elif ('throttle-one' in text and account=='A') or 'throttle-all' in text:
                status=429;headers['retry-after']='1';value={'error':{'message':'limited'}}
            elif self.headers.get('authorization')=='Bearer force-refresh':
                status=401;value={'error':{'message':'expired'}}
            elif self.path.endswith('/compact'):
                value={'id':'compact-id','object':'response.compaction','output':[{'type':'compaction','encrypted_content':'opaque-compaction'}]}
            else:
                headers['content-type']='text/event-stream';headers['x-codex-turn-state']='fixture-turn'
                events=[{'type':'response.created','response':{'id':f'resp-{serial}','model':data['model'],'status':'in_progress'}}, {'type':'response.output_item.done','output_index':0,'item':{'id':'rs-1','type':'reasoning','encrypted_content':'opaque-signature','summary':[]}}, {'type':'response.output_item.done','output_index':1,'item':{'id':'msg-1','type':'message','role':'assistant','status':'completed','content':[{'type':'output_text','text':f'OK {account}'}]}}]
                if 'broken-stream' not in text:
                    events.append({'type':'response.completed','response':{'id':f'resp-{serial}','model':data['model'],'status':'completed','output':[],'usage':{'input_tokens':2,'output_tokens':1}}})
                value=''.join('event: '+e['type']+'\r\ndata: '+json.dumps(e)+'\r\n\r\n' for e in events)
        body=(value if isinstance(value,str) else json.dumps(value)).encode()
        self.send_response(status)
        self.send_header('content-type',headers.pop('content-type','application/json'))
        self.send_header('content-length',str(len(body)))
        for k,v in headers.items():self.send_header(k,v)
        self.end_headers()
        self.wfile.write(body)

class Gateway:
    def __init__(self,directory):
        self.directory=Path(directory)
        self.config=self.directory/'gateway.toml'
        self.config.write_text(f"listen='127.0.0.1:0'\ndata_dir='{self.directory/'data'}'\nthreads=2\nmax_in_flight=64\nadmin_token_env='CODEX_SMOKE_ADMIN'\n[http]\ndrain_timeout_ms=500\n")
    def start(self):
        self.stderr=(self.directory/'gateway.log').open('a')
        self.child=subprocess.Popen([str(ROOT/'gateway/target/release/gateway'),'run','--config',str(self.config)],env={**os.environ,'CODEX_SMOKE_ADMIN':'host-admin'},stdout=subprocess.PIPE,stderr=self.stderr,text=True)
        lines=queue.Queue()
        threading.Thread(target=lambda:[lines.put(line) for line in self.child.stdout],daemon=True).start()
        ready=lines.get(timeout=10);assert 'gateway ready' in ready,ready
        self.base='http://'+ready.split('listen=',1)[1].split()[0]
    def stop(self):
        if self.child.poll() is None:
            self.child.send_signal(signal.SIGTERM)
            try:assert self.child.wait(timeout=8)==0
            finally:
                if self.child.poll() is None:self.child.kill();self.child.wait()
        self.stderr.close()
    def call(self,path,data=None,token=None,method=None,headers=None):
        h=dict(headers or {})
        if token:h['authorization']='Bearer '+token
        if isinstance(data,(dict,list)):data=json.dumps(data).encode();h['content-type']='application/json'
        req=urllib.request.Request(self.base+path,data=data,headers=h,method=method)
        try:r=urllib.request.urlopen(req,timeout=20)
        except urllib.error.HTTPError as e:r=e
        with r:
            text=r.read().decode();out=json.loads(text) if 'application/json' in r.headers.get('content-type','') else text
            return r.status,out,dict(r.headers)

def check(name, condition, value=None):
    assert condition, (name,value)
    report.append(name);print('PASS '+name,flush=True)

def main():
    upstream=ThreadingHTTPServer(('127.0.0.1',0),Upstream);upstream.daemon_threads=True
    threading.Thread(target=upstream.serve_forever,daemon=True).start()
    base='http://127.0.0.1:'+str(upstream.server_port)
    overrides={'vars':{'ADMIN_TOKEN':'plugin-admin','CLIENT_API_KEYS':[{'id':'alice','key':'client-key','models':['alias']}],'MODEL_MAP':{'alias':'backend-model'},'MODELS':['alias','other'],'UPSTREAM':base,'AUTH_BASE':base},'trace_body':False}
    with tempfile.TemporaryDirectory(prefix='codex-plugin-test-') as directory:
        g=Gateway(directory);g.start()
        try:
            status,value,_=g.call('/__gateway/plugins',(PLUGIN/'codex.zip').read_bytes(),'host-admin',headers={'content-type':'application/zip'})
            check('ZIP installs in actual QuickJS gateway',status==201,value)
            check('unconfigured inference fails closed',g.call('/v1/responses',{'model':'alias','input':'hi'})[0]==401)
            check('unconfigured admin fails closed',g.call('/codex/admin/accounts')[0]==401)
            check('runtime overrides apply',g.call('/__gateway/plugins/codex/config',overrides,'host-admin','PUT')[0]==200)
            for token in [None,'plugin-admin','host-admin','wrong']:
                check('inference rejects '+str(token),g.call('/v1/responses',{'model':'alias','input':'hi'},token)[0]==401)
            check('client key cannot administer accounts',g.call('/codex/admin/accounts',token='client-key')[0]==401)
            check('authenticated request with empty pool is 503',g.call('/v1/responses',{'model':'alias','input':'hi'},'client-key')[0]==503)
            def install(account,access='fixture-access',expires=None):
                return g.call('/codex/admin/accounts/import',{'type':'codex','access_token':access,'refresh_token':'fixture-refresh','account_id':account,'expires_at':expires if expires is not None else int(time.time()*1000)+3600000},'plugin-admin')
            check('CLIProxyAPI credential import',install('A')[0]==201)
            status,value,headers=g.call('/v1/responses',{'model':'alias','input':'你好','stream':False,'max_output_tokens':32,'temperature':.2},'client-key',headers={'x-codex-turn-state':'client-turn','cookie':'do-not-forward','chatgpt-account-id':'spoof'})
            check('ordinary JSON aggregates Codex SSE',status==200 and value['output'][1]['content'][0]['text']=='OK A',value)
            check('opaque reasoning retained',value['output'][0]['encrypted_content']=='opaque-signature')
            sent=Upstream.requests[-1];lower={k.lower():v for k,v in sent['headers'].items()}
            check('Codex native request normalization',sent['body']['stream'] is True and sent['body']['store'] is False and sent['body']['model']=='backend-model' and 'max_output_tokens' not in sent['body'] and 'temperature' not in sent['body'])
            check('OAuth replaces client authentication',lower.get('authorization')=='Bearer fixture-access' and lower.get('chatgpt-account-id')=='A' and 'cookie' not in lower)
            check('turn state forwarded both directions',lower.get('x-codex-turn-state')=='client-turn' and headers.get('x-codex-turn-state')=='fixture-turn',headers)
            status,value,_=g.call('/v1/responses',{'model':'alias','input':'hi','stream':True},'client-key')
            check('native SSE events retained',status==200 and 'response.completed' in value and 'opaque-signature' in value,value)
            check('model permission enforced',g.call('/v1/responses',{'model':'other','input':'hi'},'client-key')[0]==403)
            check('model list respects client policy',g.call('/v1/models',token='client-key')[1]['data']==[{'id':'alias','object':'model','owned_by':'codex'}])
            check('compact response preserves opaque item',g.call('/v1/responses/compact',{'model':'alias','input':'hi'},'client-key')[1]['output'][0]['encrypted_content']=='opaque-compaction')
            check('unsupported previous response is rejected',g.call('/v1/responses',{'model':'alias','input':'hi','previous_response_id':'unknown'},'client-key')[0]==400)
            check('HTTP error retains status and JSON message',g.call('/v1/responses',{'model':'alias','input':'http-fail'},'client-key')[:2]==(400,{'error':{'message':'bad fixture','type':'invalid_request_error','code':'upstream_error'}}))
            check('truncated JSON request yields 502',g.call('/v1/responses',{'model':'alias','input':'broken-stream'},'client-key')[0]==502)
            check('truncated SSE emits error event','event: error' in g.call('/v1/responses',{'model':'alias','input':'broken-stream','stream':True},'client-key')[1])
            install('A',expires=0);before=Upstream.refreshes
            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                statuses=list(pool.map(lambda _:g.call('/v1/responses',{'model':'alias','input':'hi'},'client-key')[0],range(8)))
            check('concurrent expiry shares one refresh',statuses==[200]*8 and Upstream.refreshes==before+1,(statuses,Upstream.refreshes-before))
            install('A','force-refresh');before=Upstream.refreshes
            check('401 refreshes account once before retry',g.call('/v1/responses',{'model':'alias','input':'hi'},'client-key')[0]==200 and Upstream.refreshes==before+1)
            install('B')
            output=g.call('/v1/responses',{'model':'alias','input':'throttle-one'},'client-key')
            check('429 account failover',output[0]==200 and output[1]['output'][1]['content'][0]['text']=='OK B',output)
            install('A');install('B')
            status,_,headers=g.call('/v1/responses',{'model':'alias','input':'throttle-all'},'client-key')
            check('all accounts throttled yields 429 and Retry-After',status==429 and headers.get('retry-after')=='1',headers)
            install('A');install('B')
            check('admin can disable account',g.call('/codex/admin/accounts/A/enabled',{'enabled':False},'plugin-admin','PUT')[1]['enabled'] is False)
            status,accounts,_=g.call('/codex/admin/accounts',token='plugin-admin')
            check('account listing never returns tokens',status==200 and 'access' not in json.dumps(accounts) and 'refresh' not in json.dumps(accounts))
            login=g.call('/codex/admin/accounts/login',{},'plugin-admin')[1]
            status,done,_=g.call('/codex/admin/accounts/login/'+login['id']+'/poll',{},'plugin-admin')
            check('device OAuth exchange registers account',status==200 and done['state']=='done' and done['account']['accountId']=='device-account',done)
            g.stop();g.start()
            check('account and config survive restart',len(g.call('/codex/admin/accounts',token='plugin-admin')[1])==3 and g.call('/v1/responses',{'model':'alias','input':'hi'},'client-key')[0]==200)
            overrides['vars']['CLIENT_API_KEYS'][0]['enabled']=False
            g.call('/__gateway/plugins/codex/config',overrides,'host-admin','PUT')
            check('disabled API key is rejected after config update',g.call('/v1/responses',{'model':'alias','input':'hi'},'client-key')[0]==401)
        finally:
            g.stop();upstream.shutdown();upstream.server_close()
    out=ROOT/'reviews/codex-plugin-validation';out.mkdir(exist_ok=True)
    (out/'smoke.json').write_text(json.dumps({'passed':len(report),'checks':report,'real_upstream_called':False},indent=2)+'\n')
    print(f'COMPLETE {len(report)} real-gateway checks passed; no external account used')
if __name__=='__main__':main()
