#!/usr/bin/env python3
"""Compare macOS FAT extraction by V2 and V3 CLI binaries on one prepared fixture."""
import argparse
import os,subprocess,pathlib,time,re,shutil,json,statistics,random,hashlib,csv
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--v2',type=pathlib.Path,required=True)
parser.add_argument('--v3',type=pathlib.Path,required=True)
parser.add_argument('--image',type=pathlib.Path,required=True)
parser.add_argument('--counter',type=pathlib.Path,required=True)
parser.add_argument('--output',type=pathlib.Path,required=True)
parser.add_argument('--samples',type=int,default=21)
parser.add_argument('--compiler',default='unspecified')
args=parser.parse_args()
if args.samples < 1:parser.error('--samples must be positive')
out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
image=args.image.resolve()
v2=args.v2.resolve();v3=args.v3.resolve()
expected={f'F{i:07}.TXT':b'fixture' for i in range(1000)}
expected['PAYLOAD.BIN']=bytes((i*17+i//251)%256 for i in range(131072))
configs={'v2': [str(v2),'extract'], 'v3-default':[str(v3),'fat','extract'], 'v3-no-cache':[str(v3),'fat','extract','--no-cache'],'v3-64KiB':[str(v3),'fat','extract','--read-ahead-blocks','128']}
rows=[]
for sample in range(args.samples+1):
 order=list(configs);random.Random(sample).shuffle(order)
 for label in order:
  dest=out/'extracted';assert not dest.exists()
  command=configs[label]+[str(image),'-o',str(dest)]
  start=time.perf_counter_ns();r=subprocess.run(['/usr/bin/time','-l',*command],capture_output=True,text=True);elapsed=time.perf_counter_ns()-start
  (out/f'{label}-{sample}.log').write_text(r.stdout+r.stderr)
  assert r.returncode==0,r.stderr
  rss=int(re.search(r'(\d+)\s+maximum resident set size',r.stderr)[1])
  assert {p.name:p.read_bytes() for p in dest.iterdir()}==expected,label
  shutil.rmtree(dest)
  if sample:rows.append(dict(implementation=label,sample=sample,elapsed_ns=elapsed,peak_rss_bytes=rss))
(out/'samples.json').write_text(json.dumps(rows,indent=2)+'\n')
dylib=str(args.counter.resolve())
io=[]
for label in configs:
 dest=out/'extracted';command=configs[label]+[str(image),'-o',str(dest)]
 env=dict(os.environ,DYLD_INSERT_LIBRARIES=dylib,HADRIS_IO_IMAGE=str(image))
 r=subprocess.run(command,env=env,text=True,capture_output=True,check=True)
 (out/f'{label}-syscalls.log').write_text(r.stdout+r.stderr)
 calls,requested,delivered,seeks,failures=map(int,re.search(r'IMAGE_IO,([\d,]+)',r.stderr)[1].split(','))
 assert requested==delivered and failures==0
 assert {p.name:p.read_bytes() for p in dest.iterdir()}==expected
 shutil.rmtree(dest)
 io.append(dict(implementation=label,read_calls=calls,requested_bytes=requested,seeks=seeks,failures=failures))
(out/'io.json').write_text(json.dumps(io,indent=2)+'\n')
with (out/'samples.csv').open('w',newline='') as f:
 w=csv.DictWriter(f,fieldnames=list(rows[0]));w.writeheader();w.writerows(rows)
for label in configs:
 s=[r for r in rows if r['implementation']==label];print(label,statistics.median(r['elapsed_ns'] for r in s)/1e6,statistics.median(r['peak_rss_bytes'] for r in s)/2**20,next(r for r in io if r['implementation']==label),flush=True)
(out/'metadata.json').write_text(json.dumps(dict(compiler=args.compiler,binary_sha256={label:hashlib.sha256(binary.read_bytes()).hexdigest() for label,binary in [('v2',v2),('v3',v3)]},image_sha256=hashlib.sha256(image.read_bytes()).hexdigest(),warm_cache=True,ns_includes_process_startup=True,metadata_equivalence_qualified=False),indent=2)+'\n')
