#!/usr/bin/env python3
"""Local release capacity run: fixed body/concurrency/stream duration, raw evidence."""
import sqlite3,platform,argparse,concurrent.futures,hashlib,http.client,http.server,io,json,os,pathlib,signal,subprocess,tempfile,threading,time,urllib.request,zipfile
p=argparse.ArgumentParser();p.add_argument('--binary',default='target/release/gateway');p.add_argument('--clients',type=int,default=50);p.add_argument('--seconds',type=int,default=60);p.add_argument('--body-bytes',type=int,default=478000);p.add_argument('--trace',choices=['on','off'],default='on');p.add_argument('--mode',choices=['native','transform','ir','iterate'],default='native');p.add_argument('--threads',type=int,default=1);p.add_argument('--output',required=True);a=p.parse_args()
binary_sha256=hashlib.sha256(pathlib.Path(a.binary).read_bytes()).hexdigest()
class Upstream(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def do_POST(self):
  if self.headers.get('transfer-encoding')=='chunked':
   while True:
    n=int(self.rfile.readline().strip().split(b';')[0],16)
    if not n:self.rfile.readline();break
    self.rfile.read(n);self.rfile.read(2)
  else:self.rfile.read(int(self.headers.get('content-length',0)))
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Transfer-Encoding','chunked');self.end_headers()
  try:
   for n in range(a.seconds):
    data=f'data: {n}\n\n'.encode();self.wfile.write(f'{len(data):x}\r\n'.encode()+data+b'\r\n');self.wfile.flush();time.sleep(1)
   self.wfile.write(b'0\r\n\r\n');self.wfile.flush()
  except (BrokenPipeError,ConnectionResetError):pass
 def log_message(self,*args):pass
upstream=http.server.ThreadingHTTPServer(('127.0.0.1',0),Upstream);upstream.daemon_threads=True;threading.Thread(target=upstream.serve_forever,daemon=True).start()
def request(base,path,method='GET',body=None,headers=None):
 req=urllib.request.Request(base+path,data=body,method=method,headers={'Authorization':'Bearer capacity-fixture',**(headers or {})})
 with urllib.request.urlopen(req,timeout=20) as response:return response.read()
def process_stat(pid):
 status=pathlib.Path(f'/proc/{pid}/status').read_text();rss=int(next(l.split()[1] for l in status.splitlines() if l.startswith('VmRSS:')))*1024
 stat=pathlib.Path(f'/proc/{pid}/stat').read_text().split();return {'rss':rss,'cpu_ticks':int(stat[13])+int(stat[14])}
def quantile(v,p):return sorted(v)[min(len(v)-1,int((len(v)-1)*p))] if v else None
with tempfile.TemporaryDirectory(prefix='gateway-capacity-') as tmp:
 root=pathlib.Path(tmp);cfg=root/'gateway.toml';cfg.write_text(f"listen='127.0.0.1:0'\ndata_dir='{root}/data'\nthreads={a.threads}\nmax_in_flight=128\nadmin_token_env='CAPACITY_ADMIN'\n[telemetry]\nbody={'true' if a.trace=='on' else 'false'}\n")
 stderr=(root/'stderr.log').open('w+');proc=subprocess.Popen([str(pathlib.Path(a.binary).resolve()),'run','--config',str(cfg)],stdout=subprocess.PIPE,stderr=stderr,text=True,env={**os.environ,'CAPACITY_ADMIN':'capacity-fixture'})
 try:
  ready=[]
  def read_ready():
   for line in proc.stdout:
    if 'gateway ready' in line:ready.append(line);return
  threading.Thread(target=read_ready,daemon=True).start();deadline=time.monotonic()+15
  while not ready:
   if proc.poll() is not None or time.monotonic()>deadline:raise RuntimeError('release startup failed')
   time.sleep(.02)
  address=ready[0].split('listen=')[1].split()[0];base='http://'+address
  code=f"export default {{async fetch(r,env,ctx){{let body=r.body;{'const ir=env.IR.from(r,\"openai.responses\");ir.model=\"changed\";body=env.IR.to(ir);' if a.mode=='ir' else ''}{'const ir=env.IR.from(r,\"openai.responses\");let n=0;for(const message of ir.messages)for(const block of message.content)n+=block.text?.length??0;body=env.IR.to(ir);' if a.mode=='iterate' else ''}const upstream=await ctx.fetch('http://127.0.0.1:{upstream.server_port}/',{{method:'POST',body,signal:r.signal}});return new Response({'upstream.body.pipeThrough(new TransformStream({transform(chunk,c){c.enqueue(chunk)}}))' if a.mode=='transform' else 'upstream.body'},{{headers:upstream.headers}});}}}}"
  manifest="name='capacity'\nmain='index.mjs'\nroutes=['/capacity/*']\n"+("[[unsafe.bindings]]\nname='IR'\ntype='ir'\n" if a.mode in ['ir','iterate'] else '')
  archive=io.BytesIO()
  with zipfile.ZipFile(archive,'w')as z:z.writestr('wrangler.toml',manifest);z.writestr('index.mjs',code)
  request(base,'/__gateway/plugins','POST',archive.getvalue(),{'Content-Type':'application/zip'})
  time.sleep(.3);idle=process_stat(proc.pid);body=(json.dumps({'model':'original','input':[{'role':'user','content':'x'*(max(0,a.body_bytes-100))}]}).encode() if a.mode in ['ir','iterate'] else b'x'*a.body_bytes)
  if a.mode=='iterate':
   body=json.dumps({'model':'original','input':[{'role':'user','content':'x'*1450}for _ in range(525)]}).encode();body+=b' '*max(0,a.body_bytes-len(body))
  # Keep the declared exact payload size for the 478 KB gate.
  if a.mode=='ir':body=body+b' '*(a.body_bytes-len(body))
  samples=[];running=True
  def sample():
   while running:
    try:
     sample=process_stat(proc.pid);sample['database_files']={f.name:f.stat().st_size for f in (root/'data').glob('gateway.sqlite*')};sample['at']=time.monotonic();sample['metrics']=json.loads(request(base,'/__gateway/metrics'));samples.append(sample)
    except Exception as e:samples.append({'error':str(e)})
    time.sleep(.1)
  monitor=threading.Thread(target=sample,daemon=True);monitor.start();start=time.monotonic()
  def client(_):
   begin=time.monotonic();connection=http.client.HTTPConnection(address,timeout=a.seconds+30);connection.request('POST','/capacity/',body,{'Content-Type':'application/json'});response=connection.getresponse();ttfb=time.monotonic()-begin;payload=response.read();wall=time.monotonic()-begin;connection.close();assert response.status==200,response.status;return {'ttfb':ttfb,'wall':wall,'bytes':len(payload),'hash':hashlib.sha256(payload).hexdigest()}
  with concurrent.futures.ThreadPoolExecutor(max_workers=max(1,a.clients))as pool:results=list(pool.map(client,range(a.clients)))
  elapsed=time.monotonic()-start;running=False;monitor.join(timeout=5);final=process_stat(proc.pid)
  proc.send_signal(signal.SIGTERM);proc.wait(timeout=35);assert proc.returncode==0
  files={f.name:f.stat().st_size for f in (root/'data').glob('gateway.sqlite*')};stderr.seek(0);errors=stderr.read()
  rss=max([idle['rss']]+[s['rss']for s in samples if 'rss'in s]);metrics=[s['metrics']for s in samples if 'metrics'in s]
  with sqlite3.connect(root/'data'/'gateway.sqlite')as db:
   payload=int(db.execute("SELECT (SELECT COALESCE(sum(bytes+length(CAST(value AS BLOB))),0) FROM traces)+(SELECT COALESCE(sum(length(CAST(value AS BLOB))),0) FROM logs)").fetchone()[0])
  report={'binary_sha256':binary_sha256,'effective_trace_log_payload_bytes':payload,'database_file_peaks':{name:max((s.get('database_files',{}).get(name,0)for s in samples),default=0)for name in ['gateway.sqlite','gateway.sqlite-wal','gateway.sqlite-shm']},'cpu_seconds_per_request':(final['cpu_ticks']-idle['cpu_ticks'])/os.sysconf('SC_CLK_TCK')/a.clients if a.clients else None,'scenario':vars(a),'payload_bytes':len(body),'idle_rss_bytes':idle['rss'],'peak_rss_bytes':rss,'cpu_seconds':(final['cpu_ticks']-idle['cpu_ticks'])/os.sysconf('SC_CLK_TCK'),'elapsed_seconds':elapsed,'throughput_requests_per_second':a.clients/elapsed,'ttfb':{f'p{int(q*100)}':quantile([r['ttfb']for r in results],q)for q in [.5,.95,.99]},'wall':{f'p{int(q*100)}':quantile([r['wall']for r in results],q)for q in [.5,.95,.99]},'metric_peaks':{k:max((m.get(k,0)for m in metrics),default=0)for k in ['requests','request_js_bytes','request_host_bytes','do_engines','do_js_bytes','do_host_bytes','websockets']},'trace_buffer_peak':max((m.get('telemetry',{}).get('buffer_bytes',0)for m in metrics),default=0),'database_files':files,'targets':{'idle_30mb':idle['rss']<=30_000_000,'peak_128mb':rss<=128_000_000},'results':results,'samples':samples,'stderr':errors}
  pathlib.Path(a.output).write_text(json.dumps(report,indent=2));print(json.dumps({k:v for k,v in report.items()if k not in ['samples','results','stderr']},indent=2),flush=True)
 finally:
  if proc.poll() is None:proc.kill();proc.wait()
upstream.shutdown();upstream.server_close()
