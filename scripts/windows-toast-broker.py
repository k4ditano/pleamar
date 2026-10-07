"""Owned shortcut/activation lifecycle, without sending toasts or desktop input."""
import argparse,ctypes,hashlib,json,os,re,shutil,struct,subprocess,time,uuid,winreg
from pathlib import Path

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary',type=Path,required=True)
p.add_argument('--tests',type=Path,required=True,help='Paired pleamar lib-test executable')
p.add_argument('--output',type=Path,required=True,help='New directory; retained binaries, log and report')
p.add_argument('--activation',choices=['both','com','protocol'],default='both')
a=p.parse_args()
source=a.binary.resolve(strict=True);tests=a.tests.resolve(strict=True)
helper=source.with_name('pleamar-notifications.exe')
data=helper.read_bytes();pe=struct.unpack_from('<I',data,60)[0]
assert data[pe:pe+6]==b'PE\0\0\x64\x86' and struct.unpack_from('<H',data,pe+92)[0]==2,'COM helper must be native x64 GUI subsystem'
out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
folder=out/'broker ñ 海';folder.mkdir()
engine=folder/'pleamar.exe';broker=folder/helper.name
shutil.copy2(source,engine);shutil.copy2(helper,broker)
for name in ['vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll']:
    if (source.parent/name).is_file():shutil.copy2(source.parent/name,folder/name)
def fnv(value):
    h=0xcbf29ce484222325
    for byte in value.encode('utf-8'):h=((h^byte)*0x100000001b3)&0xffffffffffffffff
    return h
app=f'org.pleamar.desktop.{fnv(str(engine).replace("/",chr(92)).lower()):016x}'
clsid=uuid.UUID(int=0x61e6ee1c8b1a41750000000000000000|fnv(app))
key='Software\\Classes\\CLSID\\{'+str(clsid)+'}\\LocalServer32'
protocol='Software\\Classes\\pleamar-notify-'+f'{fnv(app):016x}'
protocol_command=protocol+'\\shell\\open\\command'
def protocol_value(path=protocol_command,name=None):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,path) as entry:return winreg.QueryValueEx(entry,name)[0]
    except FileNotFoundError:return None
def server():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,key,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY) as entry:
            return winreg.QueryValueEx(entry,None)[0]
    except FileNotFoundError:return None
assert server() is None,'Refuse to change an existing COM registration'
assert protocol_value() is None and protocol_value(protocol,'URL Protocol') is None,'Refuse to change an existing protocol'
user=ctypes.WinDLL('user32');user.GetForegroundWindow.restype=ctypes.c_void_p
initial=user.GetForegroundWindow()
report=dict(passed=False,actual_toast_click=False,toast_published=False,physical_input_sent=False,
    activation_test=a.activation,engine_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),broker_sha256=hashlib.sha256(data).hexdigest(),stages=[])
def run(command,env=None,timeout=30):
    r=subprocess.run(list(map(str,command)),env=env,capture_output=True,text=True,encoding='utf-8',errors='replace',
        timeout=timeout,creationflags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS)
    with (out/'commands.log').open('a',encoding='utf-8') as f:f.write(r.stdout+r.stderr)
    assert r.returncode==0,(r.returncode,r.stdout,r.stderr)
    return r
def create_shortcut(path,target):
    # Use IShellLinkW: WScript.Shell's ANSI TargetPath rejects these Unicode paths.
    ole=ctypes.WinDLL('ole32')
    ole.CoInitializeEx.argtypes=[ctypes.c_void_p,ctypes.c_uint32];ole.CoInitializeEx.restype=ctypes.c_int32
    ole.CoCreateInstance.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_void_p]
    ole.CoCreateInstance.restype=ctypes.c_int32
    assert ole.CoInitializeEx(None,0)>=0
    def guid(value):return (ctypes.c_ubyte*16).from_buffer_copy(uuid.UUID(value).bytes_le)
    def call(obj,index,types,*args):
        address=ctypes.cast(obj,ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents[index]
        result=ctypes.WINFUNCTYPE(ctypes.c_int32,ctypes.c_void_p,*types)(address)(obj,*args)
        assert result>=0,f'Shortcut COM operation failed: {result & 0xffffffff:08x}'
    link,file=ctypes.c_void_p(),ctypes.c_void_p()
    try:
        assert ole.CoCreateInstance(ctypes.byref(guid('00021401-0000-0000-c000-000000000046')),None,1,
            ctypes.byref(guid('000214f9-0000-0000-c000-000000000046')),ctypes.byref(link))>=0
        call(link,20,(ctypes.c_wchar_p,),str(target))
        call(link,11,(ctypes.c_wchar_p,),'--version')
        call(link,0,(ctypes.c_void_p,ctypes.c_void_p),ctypes.byref(guid('0000010b-0000-0000-c000-000000000046')),ctypes.byref(file))
        call(file,6,(ctypes.c_wchar_p,ctypes.c_int32),str(path),1)
    finally:
        if file:call(file,2,())
        if link:call(link,2,())
        ole.CoUninitialize()
def owned_processes():
    # Query only this helper's process name, then verify its full image path.
    command='@([Diagnostics.Process]::GetProcessesByName("pleamar-notifications") | ForEach-Object { try { if ($_.MainModule.FileName -eq $env:PLEAMAR_TEST_BROKER) { $_.Id } } catch {} finally { $_.Dispose() } }) | ConvertTo-Json -Compress'
    answer=run([host,'-NoProfile','-NonInteractive','-Command',command],dict(os.environ,PLEAMAR_TEST_BROKER=str(broker))).stdout.strip()
    return json.loads(answer) if answer else []
host=Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'
shortcut=folder/'Owned test.lnk'
try:
    create_shortcut(shortcut,engine)
    run([engine,'--register-notification-shortcut',shortcut])
    assert server()==f'"{broker}"'
    expected_protocol=f'"{broker}" --activate-notification "%1"'
    assert protocol_value()==expected_protocol and protocol_value(protocol,'URL Protocol')==''
    run([engine,'--check-notification-shortcut',shortcut])
    report['stages'].append('Unicode shortcut, AUMID/CLSID and exact quoted native server verified')
    # No scene or endpoint exists for this nonce. CoCreateInstance must start the
    # installed GUI-subsystem helper; both stale activations are discarded.
    assert not owned_processes()
    if a.activation in ['both','com']:
        activation=run([tests,'--exact','platform::windows_toast_actions::tests::activation_child','--ignored','--nocapture'],
            dict(os.environ,PLEAMAR_TEST_TOAST_APP=app,PLEAMAR_TEST_TOAST_TOKEN=f'v1.{uuid.uuid4().hex}.{uuid.uuid4().hex}'))
        assert 'activation_child ... ok' in activation.stdout and '1 passed' in activation.stdout,'Paired activation test did not execute'
        report['stages'].append('real COM startup, wrong-app/malformed rejection and duplicate expired activations completed')
    if a.activation in ['both','protocol']:
        activation=run([tests,'--exact','platform::windows_toast_actions::tests::protocol_activation_child','--ignored','--nocapture'],
            dict(os.environ,PLEAMAR_TEST_TOAST_APP=app,PLEAMAR_TEST_TOAST_ENGINE=str(engine)))
        assert 'protocol_activation_child ... ok' in activation.stdout and '1 passed' in activation.stdout,'Protocol test did not execute'
        report['idle_receiver_cpu_cycles']=[int(value) for value in re.findall(r'idle receiver cycle [0-9]+: ([0-9]+) CPU cycles during 350 ms',activation.stdout)]
        assert len(report['idle_receiver_cpu_cycles'])==2,'Both wake/replay/idle cycles must execute'
        report['idle_receiver_sample_ms']=350
        for bad in [['--activate-notification'],['--activate-notification','https://example.invalid'],['--activate-notification','wrong','extra']]:
            result=subprocess.run([str(broker),*bad],timeout=5,creationflags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS)
            assert result.returncode==1,'Malformed helper arguments must be rejected'
        report['stages'].append('real protocol startup and delivery, duplicate rejection and bounded malformed arguments passed')
    until=time.monotonic()+20
    while owned_processes():
        assert time.monotonic()<until,'Owned COM helper did not exit'
        time.sleep(.25)
    report['stages'].append('windowless helper exited without starting a scene')
    # An unrelated replacement server must never be removed or overwritten.
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,key,0,winreg.KEY_SET_VALUE|winreg.KEY_WOW64_64KEY) as entry:
        winreg.SetValueEx(entry,None,0,winreg.REG_SZ,'owned test replacement')
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,protocol_command,0,winreg.KEY_SET_VALUE) as entry:
        winreg.SetValueEx(entry,None,0,winreg.REG_SZ,'owned protocol replacement')
    run([engine,'--unregister-notification-publisher']);assert server()=='owned test replacement'
    assert protocol_value()=='owned protocol replacement'
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,key,0,winreg.KEY_SET_VALUE|winreg.KEY_WOW64_64KEY) as entry:
        winreg.SetValueEx(entry,None,0,winreg.REG_SZ,f'"{broker}"')
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,protocol_command,0,winreg.KEY_SET_VALUE) as entry:
        winreg.SetValueEx(entry,None,0,winreg.REG_SZ,expected_protocol)
        winreg.SetValueEx(entry,'unrelated',0,winreg.REG_SZ,'preserve')
    run([engine,'--unregister-notification-publisher']);assert server() is None
    assert protocol_value() is None and protocol_value(protocol_command,'unrelated')=='preserve'
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,protocol_command,0,winreg.KEY_SET_VALUE) as entry:winreg.DeleteValue(entry,'unrelated')
    for path in [protocol_command,protocol+'\\shell\\open',protocol+'\\shell',protocol]:winreg.DeleteKey(winreg.HKEY_CURRENT_USER,path)
    run([engine,'--unregister-notification-publisher'])
    report['stages'].append('unregister preserves replacement server, removes exact owner, and is idempotent')
    report['passed']=True
finally:
    if protocol_value()=='owned protocol replacement':
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,protocol_command,0,winreg.KEY_SET_VALUE) as entry:winreg.SetValueEx(entry,None,0,winreg.REG_SZ,expected_protocol)
    if server()=='owned test replacement':
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,key,0,winreg.KEY_SET_VALUE|winreg.KEY_WOW64_64KEY) as entry:
            winreg.SetValueEx(entry,None,0,winreg.REG_SZ,f'"{broker}"')
    run([engine,'--unregister-notification-publisher'])
    report['registration_removed']=server() is None
    report['protocol_removed']=protocol_value() is None and protocol_value(protocol,'URL Protocol') is None
    report['foreground_unchanged']=user.GetForegroundWindow()==initial
    (out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps(report,indent=2))
