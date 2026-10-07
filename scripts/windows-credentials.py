"""Exercise the native credential service from Luau with an owned dummy name."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import uuid

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args()
assert os.name=='nt'
out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
scene=out/('credential-native-'+uuid.uuid4().hex+'.plm')
scene.write_text('''scene CredentialTest {
 permissions { services: "credentials.*" }
 surface { kind: window; size: 40, 40; keyboard: none; screens: "__credential_test_absent__" }
}''',encoding='utf-8')
scene.with_suffix('.luau').write_text('''
local name="Owned test café 日本語"
assert(#sys.ask("credentials.list")==0,"test owner unexpectedly exists")
local ok,reason=pcall(function()
    sys.call("credentials.set",name,"Dummy fixture · café 🪼")
    local list=sys.ask("credentials.list")
    assert(#list==1 and list[1]==name)
    assert(not pcall(sys.ask,"credentials.read",name),"values must not leave the native service")
    sys.call("credentials.set",name,"Replacement dummy")
    assert(#sys.ask("credentials.list")==1)
end)
sys.call("credentials.remove",name)
assert(#sys.ask("credentials.list")==0)
assert(ok,reason)
log("PASS: native Luau credential names, writes, overwrite, restricted read and cleanup")
''',encoding='utf-8')
env=dict(os.environ,APPDATA=str(out/'roaming'),LOCALAPPDATA=str(out/'local'),
    PLEAMAR_NO_RELAUNCH='1',PLEAMAR_SOCKET_DIR='credentials-'+uuid.uuid4().hex)
result=subprocess.run([str(a.binary.resolve()),'--scene',str(scene),'--no-hud','--stall','0','--seconds','2'],
    env=env,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=30,
    creationflags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS)
trace=result.stdout+result.stderr
(out/'runtime.log').write_text(trace,encoding='utf-8')
assert result.returncode==0 and 'PASS: native Luau credential' in trace,trace
assert 'first frame' not in trace and 'panicked' not in trace,trace
(out/'report.json').write_text(json.dumps({'passed':True,'native_vault':True,'luau':True,
    'owner':scene.stem,'dummy_only':True,'cleanup_confirmed':True,'physical_input':False},indent=2)+'\n',encoding='utf-8')
print('PASS: real Luau/Windows credential service with isolated dummy data and confirmed cleanup')
