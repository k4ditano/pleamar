from pathlib import Path
import ctypes as C, ctypes.wintypes as W, subprocess, os, time, json, hashlib
from PIL import Image

import argparse
parser=argparse.ArgumentParser(description="Exercise thumbnails only with owned passive DISPLAY2 windows (requires Pillow).")
parser.add_argument("--binary",type=Path,required=True)
parser.add_argument("--harness",type=Path,required=True,help="pleamar library test executable containing native_scene_capture")
parser.add_argument("--output",type=Path,required=True,help="new output directory")
args=parser.parse_args()
binary=args.binary.resolve(strict=True);harness=args.harness.resolve(strict=True)
out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
flags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS
U=C.WinDLL('user32',use_last_error=True);G=C.WinDLL('gdi32');K=C.WinDLL('kernel32')
def api(lib,name,args,ret):
    f=getattr(lib,name);f.argtypes=args;f.restype=ret;return f
api(U,'SetProcessDpiAwarenessContext',[W.HANDLE],W.BOOL)(W.HANDLE(-4))
api(U,'CreateWindowExW',[W.DWORD,W.LPCWSTR,W.LPCWSTR,W.DWORD,C.c_int,C.c_int,C.c_int,C.c_int,W.HWND,W.HMENU,W.HINSTANCE,W.LPVOID],W.HWND)
api(U,'ShowWindow',[W.HWND,C.c_int],W.BOOL);api(U,'DestroyWindow',[W.HWND],W.BOOL)
api(U,'EnableWindow',[W.HWND,W.BOOL],W.BOOL)
api(U,'SetWindowPos',[W.HWND,W.HWND,C.c_int,C.c_int,C.c_int,C.c_int,W.UINT],W.BOOL)
api(U,'InvalidateRect',[W.HWND,W.LPVOID,W.BOOL],W.BOOL)
api(U,'GetForegroundWindow',[],W.HWND)
api(U,'GetClientRect',[W.HWND,C.POINTER(W.RECT)],W.BOOL)
api(U,'PeekMessageW',[C.POINTER(W.MSG),W.HWND,W.UINT,W.UINT,W.UINT],W.BOOL)
api(U,'TranslateMessage',[C.POINTER(W.MSG)],W.BOOL);api(U,'DispatchMessageW',[C.POINTER(W.MSG)],C.c_ssize_t)
api(U,'DefWindowProcW',[W.HWND,W.UINT,W.WPARAM,W.LPARAM],C.c_ssize_t)
api(G,'CreateSolidBrush',[W.DWORD],W.HBRUSH);api(G,'DeleteObject',[W.HANDLE],W.BOOL)
api(U,'FillRect',[W.HDC,C.POINTER(W.RECT),W.HBRUSH],C.c_int)
class PAINT(C.Structure):
    _fields_=[('hdc',W.HDC),('erase',W.BOOL),('rect',W.RECT),('restore',W.BOOL),('update',W.BOOL),('reserved',W.BYTE*32)]
api(U,'BeginPaint',[W.HWND,C.POINTER(PAINT)],W.HDC);api(U,'EndPaint',[W.HWND,C.POINTER(PAINT)],W.BOOL)
PROC=C.WINFUNCTYPE(C.c_ssize_t,W.HWND,W.UINT,W.WPARAM,W.LPARAM)
colors={};painted={}
@PROC
def procedure(hwnd,msg,w,l):
    if msg==15:
        painted[hwnd]=colors.get(hwnd,0x20c060)
        p=PAINT();dc=U.BeginPaint(hwnd,C.byref(p));brush=G.CreateSolidBrush(colors.get(hwnd,0x20c060))
        r=W.RECT();U.GetClientRect(hwnd,C.byref(r));U.FillRect(dc,C.byref(r),brush)
        G.DeleteObject(brush);U.EndPaint(hwnd,C.byref(p));return 0
    return U.DefWindowProcW(hwnd,msg,w,l)
class WNDCLASS(C.Structure):
    _fields_=[('style',W.UINT),('proc',PROC),('extra',C.c_int),('window_extra',C.c_int),('instance',W.HINSTANCE),
              ('icon',W.HICON),('cursor',W.HANDLE),('brush',W.HBRUSH),('menu',W.LPCWSTR),('name',W.LPCWSTR)]
api(K,'GetModuleHandleW',[W.LPCWSTR],W.HMODULE)
api(U,'RegisterClassW',[C.POINTER(WNDCLASS)],W.WORD)
api(U,'UnregisterClassW',[W.LPCWSTR,W.HINSTANCE],W.BOOL)
instance=K.GetModuleHandleW(None);class_name='pleamar-thumbnails-owned-'+str(os.getpid())
wc=WNDCLASS();wc.proc=procedure;wc.instance=instance;wc.name=class_name
assert U.RegisterClassW(C.byref(wc))
def pump():
    msg=W.MSG()
    while U.PeekMessageW(C.byref(msg),None,0,0,1):U.TranslateMessage(C.byref(msg));U.DispatchMessageW(C.byref(msg))
def pause(seconds):
    until=time.monotonic()+seconds
    while time.monotonic()<until:pump();time.sleep(.01)
def run(argv,env,timeout=15):
    p=subprocess.Popen(list(map(str,argv)),env=env,creationflags=flags,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    until=time.monotonic()+timeout
    while p.poll() is None:
        if time.monotonic()>until:p.kill();p.wait();raise TimeoutError(str(argv))
        pause(.01)
    a,b=p.communicate()
    if p.returncode:raise RuntimeError(b.decode('utf-8',errors='replace'))
    return a.decode('utf-8').strip()

env=dict(os.environ,PLEAMAR_NO_RELAUNCH='1',PLEAMAR_SOCKET_DIR='thumbnail-'+str(os.getpid()),
         PLEAMAR_SCENE_TEST_PASSIVE='1',PLEAMAR_SCENE_TEST_SECONDS='240')
class MONITORINFO(C.Structure):
    _fields_=[('size',W.DWORD),('monitor',W.RECT),('work',W.RECT),('flags',W.DWORD),('name',W.WCHAR*32)]
MONITORPROC=C.WINFUNCTYPE(W.BOOL,W.HANDLE,W.HDC,C.POINTER(W.RECT),W.LPARAM)
api(U,'EnumDisplayMonitors',[W.HDC,W.LPVOID,MONITORPROC,W.LPARAM],W.BOOL)
api(U,'GetMonitorInfoW',[W.HANDLE,C.POINTER(MONITORINFO)],W.BOOL)
screens=[]
@MONITORPROC
def monitor(handle,dc,rect,data):
    info=MONITORINFO();info.size=C.sizeof(info)
    if U.GetMonitorInfoW(handle,C.byref(info)):
        r=info.work
        screens.append(dict(name=info.name,primary=bool(info.flags&1),
            work=dict(x=r.left,y=r.top,width=r.right-r.left,height=r.bottom-r.top)))
    return True
assert U.EnumDisplayMonitors(None,None,monitor,0)
screen=next(m for m in screens if m['name']==r"\\.\DISPLAY2" and not m['primary'])
work=screen['work'];foreground=U.GetForegroundWindow();owned=[];p=None
report={'passed':False,'physical_input':False,'monitor':screen['name'],'primary':False,
        'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'stages':[]}
scene=out/'native-thumbnails.plm'
source="""scene NativeThumbnails {
    surface { size: 920, 380; anchor: center; keyboard: none; reserve: 0 }
    permissions { services: "thumbnails", "thumbnails.*" }
    model shots max 2 { id: text; title: text; picture: image 400, 250; stale: bool }
    fact seen = 0
    fact stale = 0
    text picture0 = ""
    text picture1 = ""
    text status = "Live native thumbnails"
    event live ->
    event still ->
    event stop ->
    box { from: 0, 0; size: 920, 380; color: #101820 }
    text "Windows · miniaturas nativas" { at: 20, 16; size: 22; color: #eeeeee }
    row { at: 20, 58; gap: 40
        for s in shots {
            column { gap: 6
                image s.picture { size: 400, 250 }
                text "{s.title}" { size: 14; color: #eeeeee }
            }
        }
    }
    text "{status}" { at: 20, 350; size: 16; color: #9ed6bd }
}
"""
scene.write_text(source,encoding='utf-8')
logic=r"""local rows = {}
local mode = "live"
local key = ""
local function request()
    local ids = {}
    for _, row in rows do table.insert(ids,row.id) end
    local nextkey=mode..table.concat(ids,",")
    if nextkey==key then return end
    key=nextkey
    sys.call("thumbnails.live", if mode=="live" then ids else nil)
    sys.call("thumbnails.want", if mode=="still" then ids else nil)
end
sys.watch("thumbnails",function(state)
    rows={}
    for _,row in state.list or {} do
        if string.sub(row.title,1,#"OWNED_TITLE")=="OWNED_TITLE" then table.insert(rows,row) end
    end
    table.sort(rows,function(a,b) return a.title<b.title end)
    model.shots=rows
    fact.seen=#rows
    local stale=0
    for _,row in rows do if row.stale then stale+=1 end end
    fact.stale=stale
    text.picture0=rows[1] and rows[1].picture or ""
    text.picture1=rows[2] and rows[2].picture or ""
    request()
end)
for _, name in {"live","still","stop"} do
    on(name,function() mode=name;text.status=name;request() end)
end
""".replace('OWNED_TITLE',class_name)
scene.with_suffix('.luau').write_text(logic,encoding='utf-8')
def ask(command): return run([binary,'--say',scene.stem,command],env)
def until(predicate,label,seconds=25):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        assert p is None or p.poll() is None,label+' harness exited'
        try:
            if predicate():return
        except (RuntimeError,FileNotFoundError):pass
        pause(.07)
    raise TimeoutError(label)
def capture(label):
    (out/'request').write_text(label)
    until(lambda:(out/'captured').read_text()==label,label)
    report['stages'].append(label)
    return Image.open(out/(label+'.png')).convert('RGB')
def color_count(image,color):
    return sum(n for n,c in image.getcolors(image.width*image.height) if max(abs(c[k]-color[k]) for k in range(3))<=3)
try:
    for i in range(2):
        x=work['x']+30+i*480;y=work['y']+30
        assert x+450<=work['x']+work['width'] and y+300<=work['y']+work['height']
        h=U.CreateWindowExW(0x08040020,class_name,class_name+' '+str(i),0x00cf0000,x,y,450,300,None,None,instance,None)
        assert h;owned.append(h);colors[h]=0x20c060;U.EnableWindow(h,False);U.ShowWindow(h,4);pump()
    run([binary,'--check',scene],env)
    env.update(PLEAMAR_SCENE_TEST_BINARY=str(binary),PLEAMAR_SCENE_TEST_FILE=str(scene),PLEAMAR_SCENE_TEST_OUTPUT=str(out))
    with (out/'harness.log').open('w',encoding='utf-8') as log:
        p=subprocess.Popen([str(harness),'--ignored','--exact','platform::windows_desktop::scene_tests::native_scene_capture','--nocapture'],env=env,creationflags=flags,stdout=log,stderr=log)
        until(lambda:ask('get seen')=='2' and ask('get picture0').startswith('thumbnails:') and ask('get picture1').startswith('thumbnails:'),'two real live pictures')
        pause(.5);assert color_count(capture('01-live'),(96,192,32))>40000
        old=ask('get picture0');colors[owned[0]]=0xd03080;U.InvalidateRect(owned[0],None,False)
        until(lambda:ask('get picture0')!=old,'live repaint')
        assert color_count(capture('02-repaint'),(128,48,208))>20000
        old=ask('get picture0');assert U.SetWindowPos(owned[0],None,0,0,650,400,2|4|16)
        until(lambda:ask('get picture0')!=old,'live resize')
        assert color_count(capture('03-resize'),(128,48,208))>20000
        # Only our disabled, non-activating source is minimized/restored.
        U.ShowWindow(owned[0],7)
        until(lambda:ask('get stale')=='1','minimized source marked stale')
        capture('04-minimized')
        U.ShowWindow(owned[0],4)
        until(lambda:ask('get stale')=='0','restored source resumes')
        ask('emit still')
        until(lambda:'.png?' in ask('get picture0') and '.png?' in ask('get picture1'),'live to PNG')
        paths=[Path(ask('get picture'+str(i)).split('?')[0]) for i in range(2)]
        assert all(p.is_file() for p in paths)
        assert color_count(capture('04-still'),(128,48,208))>20000
        old=ask('get picture0');colors[owned[0]]=0x4080e0;U.InvalidateRect(owned[0],None,False)
        until(lambda:ask('get picture0')!=old,'still repaint')
        ask('emit stop');pause(1);old=ask('get picture0')
        colors[owned[0]]=0xd03080;U.InvalidateRect(owned[0],None,False);pause(1)
        assert ask('get picture0')==old,'capture continued after release'
        assert color_count(capture('05-stopped'),(224,128,64))>20000
        scene.with_suffix('.luau').write_text(logic+'\n-- native reload\n',encoding='utf-8')
        until(lambda:ask('get picture0').startswith('thumbnails:'),'reload live ownership')
        assert color_count(capture('06-reloaded'),(128,48,208))>20000
        U.DestroyWindow(owned.pop());until(lambda:ask('get seen')=='1','source closure')
        capture('07-closed')
        (out/'request').write_text('quit');until(lambda:(out/'capture-result.json').exists(),'normal scene exit')
        assert p.wait(timeout=10)==0
    report.update(passed=True,actual_wgc_to_d3d12=True,source_updates=True,source_resize=True,
        source_closure=True,scene_reload=True,minimize_restore=True,live_to_file=True,capture_stopped=True,cache_cleaned=all(not path.exists() for path in paths),
        native=json.loads((out/'capture-result.json').read_text()),foreground_same_at_end=U.GetForegroundWindow()==foreground)
    assert report['cache_cleaned']
except Exception as error:
    report['error']=str(error);raise
finally:
    if p is not None and p.poll() is None:
        (out/'request').write_text('quit')
        try:p.wait(timeout=15)
        except subprocess.TimeoutExpired:p.kill();p.wait()
    for h in owned:U.DestroyWindow(h)
    U.UnregisterClassW(class_name,instance)
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(str(out));print(json.dumps(report,indent=2))
