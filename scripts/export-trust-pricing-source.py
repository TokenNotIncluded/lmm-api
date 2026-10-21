#!/usr/bin/env python3
"""Read original cc52 NET/audit functions in ONE readonly restored-source tx.
No startup/restore/migration/target DB writes. All material stays private.
Requires the independently verified source receipt and frozen importer helper.
"""
from pathlib import Path
import argparse,fcntl,hashlib,importlib.util,io,json,os,re,subprocess,sys,tarfile
os.umask(0o077)
a=argparse.ArgumentParser(description=__doc__)
a.add_argument('--source-receipt',required=True,type=Path)
a.add_argument('--source-lifecycle',required=True,type=Path)
a.add_argument('--readonly-helper',required=True,type=Path)
a.add_argument('--helper-sha256',required=True)
a.add_argument('--archive-sha256',required=True)
a.add_argument('--source-cutoff',required=True,type=int)
a.add_argument('--output',required=True,type=Path)
a.add_argument('--heavy-lock',required=True,type=Path)
x=a.parse_args();root=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def require(b,code):
 if not b:raise SystemExit(code)
require(sha(x.readonly_helper)==x.helper_sha256,'HELPER_HASH_MISMATCH')
require(not x.output.exists() and x.output.is_absolute(),'FRESH_PRIVATE_OUTPUT_REQUIRED');x.output.mkdir(mode=0o700)
r=json.loads(x.source_receipt.read_text());require(r['archive']['sha256']==x.archive_sha256,'ARCHIVE_IDENTITY_MISMATCH')
require(0<x.source_cutoff<253402300800,'INVALID_CUTOFF')
lifecycle=json.loads(x.source_lifecycle.read_text());pid=lifecycle['postmaster_pid'];proc=Path(f'/proc/{pid}')
require(type(pid)is int and pid>1 and proc.is_dir(),'SOURCE_PROCESS_NOT_ALIVE')
require(str(lifecycle['data']).encode()in(proc/'cmdline').read_bytes(),'SOURCE_DATA_PROCESS_MISMATCH')
require(int((Path(lifecycle['data'])/'postmaster.pid').read_text().splitlines()[0])==pid,'SOURCE_PID_MISMATCH')
start_ticks=(proc/'stat').read_text().rsplit(')',1)[1].split()[19]
require(lifecycle['port']==r['readonly_reader']['port'] and str(lifecycle['socket'])==r['readonly_reader']['socket_directory'],'SOURCE_LISTENER_MISMATCH')
spec=importlib.util.spec_from_file_location('reviewed_readonly',x.readonly_helper);I=importlib.util.module_from_spec(spec);sys.modules[spec.name]=I;spec.loader.exec_module(I)
ident=json.loads(I.psql(r['readonly_reader'],'BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL row_security=off;\n'+I.IDENTITY_SQL+'\nROLLBACK;',readonly=True,private_dir=x.output,label='source-preflight'))
I.verify_instance(ident,r['readonly_reader'],r['source_instance_identity'],source=True)
rev='cc52aa195c6f2a01d703a1a407be73c31b89867d'
raw=subprocess.check_output(['git','archive',rev,'apps/api-go'],cwd=root)
with tarfile.open(fileobj=io.BytesIO(raw))as t:
 require(all(not p.issym() and not p.islnk() and (p.name.startswith('apps/api-go/')or p.name in ['apps','apps/api-go'])for p in t.getmembers()),'SOURCE_ARCHIVE_INVALID')
 t.extractall(x.output/'source',filter='data')
c=x.output/'source/apps/api-go';original={str(p.relative_to(c)):sha(p)for p in c.rglob('*')if p.is_file()}
(c/'model/trust_source_export.go').write_bytes((root/'scripts/trust_pricing/source_export_model.go.txt').read_bytes())
d=c/'cmd/trust-source-export';d.mkdir(parents=True);(d/'main.go').write_bytes((root/'scripts/trust_pricing/source_export_main.go.txt').read_bytes())
env={k:v for k,v in os.environ.items()if not k.startswith('PG')};env.update({'GOPROXY':'off','GOSUMDB':'off','GOTOOLCHAIN':'local','GOMAXPROCS':'2'})
with x.heavy_lock.open('a')as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 with (x.output/'compile.private.log').open('w')as log:
  build=subprocess.run(['go','build','-p=2','-mod=readonly','-o',str(x.output/'source-export'),'./cmd/trust-source-export'],cwd=c,env=env,stdout=log,stderr=subprocess.STDOUT)
 require(build.returncode==0,'SOURCE_ADAPTER_BUILD_FAILED')
require(all(sha(c/p)==d for p,d in original.items()),'ORIGINAL_SOURCE_DRIFT')
require(re.fullmatch('[a-z_][a-z_0-9]*',r['source']['schema'])is not None,'SOURCE_NAMESPACE_INVALID')
env.update(I.socket_connection(r['readonly_reader']));env['PGOPTIONS']=I.psql_options(r['readonly_reader'],readonly=True)+' -c search_path='+r['source']['schema']+',pg_catalog';env['TRUST_EXPORT_PATH']=str(x.output/'basis.private.json')
with (x.output/'runtime.private.log').open('w')as log:
 run=subprocess.run([str(x.output/'source-export')],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=180)
require(run.returncode==0,'SOURCE_ADAPTER_EXECUTION_FAILED')
basis=json.loads((x.output/'basis.private.json').read_text());require(basis['transaction_read_only']=='on' and basis['transaction_isolation']=='repeatable read','READONLY_MODE_MISMATCH')
basis.update({'source_commit':rev,'source_dump_sha256':x.archive_sha256,'source_cutoff':x.source_cutoff,'source_receipt_sha256':sha(x.source_receipt),'original_manifest_sha256':hashlib.sha256(json.dumps(original,sort_keys=True).encode()).hexdigest()})
(x.output/'basis.private.json').write_text(json.dumps(basis,indent=2)+'\n')
require(all(sha(c/p)==d for p,d in original.items()),'ORIGINAL_SOURCE_DRIFT')
require((proc/'stat').read_text().rsplit(')',1)[1].split()[19]==start_ticks,'SOURCE_PROCESS_CHANGED')
(x.output/'original-source-manifest.json').write_text(json.dumps(original,indent=2)+'\n')
(x.output/'receipt.json').write_text(json.dumps({'basis_sha256':sha(x.output/'basis.private.json'),'binary_sha256':sha(x.output/'source-export'),'original_source_drift':[],'read_only':True,'users':len(basis['users']),'projection_unknown':sum(not u['projection_known']for u in basis['users'])},indent=2)+'\n')
print(json.dumps({'users':len(basis['users']),'private_basis_sha256':sha(x.output/'basis.private.json'),'database_writes':False}))
