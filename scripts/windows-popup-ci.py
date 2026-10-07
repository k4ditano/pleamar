"""Native popup bounds and owner lifecycle, exclusively on disposable Windows CI."""
from pathlib import Path
import argparse, ctypes as C, hashlib, importlib.util, json, os, subprocess, sys, time
from ctypes import wintypes as W

spec=importlib.util.spec_from_file_location('owned_popup_desktop',Path(__file__).with_name('windows-agent-guard-ci.py'))
owned=importlib.util.module_from_spec(spec);spec.loader.exec_module(owned)

SOURCE='''scene PopupPlacementCI {
    surface { size: 360, 240; kind: window; title: "Owned popup placement CI" }
    fact opened = false
    box { from: 0, 0; size: 360, 240; color: #23313b }
    text "Owned parent" { at: 20, 30; size: 20; color: #ffffff }
    popup menu { at: 200, 120; size: 260, 150; open: opened
        box { from: 0, 0; size: 260, 150; corner: 12; color: #174c42 }
        text "Popup · España" { at: 12, 20; size: 20; color: #ffffff }
        text "Follows its owner" { at: 12, 80; size: 16; color: #c3e7d4 }
    }
}'''

class MonitorInfo(C.Structure):
    _fields_=[('size',W.DWORD),('monitor',W.RECT),('work',W.RECT),('flags',W.DWORD)]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if (sys.platform!='win32' or os.environ.get('GITHUB_ACTIONS')!='true'
        or os.environ.get('RUNNER_ENVIRONMENT')!='github-hosted' or os.environ.get('PLEAMAR_CI_POPUP')!='1'):
        raise RuntimeError('Visible popup fixture requires its explicit disposable GitHub-hosted Windows step')
    binary=args.binary.resolve(strict=True);out=args.output.resolve();temporary=Path(os.environ['RUNNER_TEMP']).resolve()
    assert out!=temporary and out.is_relative_to(temporary);out.mkdir(parents=True,exist_ok=False)
    scene=out/'Popup ñ.plm';scene.write_text(SOURCE,encoding='utf-8')
    env=dict(os.environ,PLEAMAR_SOCKET_DIR=f'popup-ci-{os.getpid()}',PLEAMAR_NO_RELAUNCH='1',PLEAMAR_TEST_WINDOWS='1')
    flags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS
    desktop=owned.Desktop();u=desktop.user
    for name,result,arguments in [
        ('GetWindowRect',W.BOOL,[W.HWND,C.POINTER(W.RECT)]),
        ('GetWindow',W.HWND,[W.HWND,W.UINT]),
        ('GetDpiForWindow',W.UINT,[W.HWND]),
        ('MonitorFromWindow',W.HANDLE,[W.HWND,W.DWORD]),
        ('MonitorFromPoint',W.HANDLE,[W.POINT,W.DWORD]),
        ('GetMonitorInfoW',W.BOOL,[W.HANDLE,C.POINTER(MonitorInfo)]),
        ('SetWindowPos',W.BOOL,[W.HWND,W.HWND,C.c_int,C.c_int,C.c_int,C.c_int,W.UINT]),
        ('ShowWindow',W.BOOL,[W.HWND,C.c_int]),
    ]:
        function=getattr(u,name);function.restype=result;function.argtypes=arguments
    process=None;report=dict(passed=False,environment='github-hosted',physical_input=False,only_owned_windows=True,images=[],placements=[])
    def run(*arguments):
        r=subprocess.run([str(binary),*arguments],env=env,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=15,creationflags=flags)
        assert r.returncode==0 and not r.stdout.lstrip().startswith('?'),r.stdout+r.stderr
        return r.stdout.strip()
    def ask(command):return run('--say',scene.stem,command)
    def until(predicate):
        deadline=time.monotonic()+30
        while time.monotonic()<deadline:
            assert process.poll() is None,'owned scene exited unexpectedly'
            try:
                value=predicate()
                if value:return value
            except (AssertionError,OSError):pass
            time.sleep(.1)
        raise TimeoutError('owned popup did not reach its expected native state')
    def rect(hwnd):
        r=W.RECT();assert u.GetWindowRect(hwnd,C.byref(r));return [r.left,r.top,r.right,r.bottom]
    def info(handle):
        result=MonitorInfo();result.size=C.sizeof(result);assert u.GetMonitorInfoW(handle,C.byref(result));return result
    def expected(parent,popup):
        scale=u.GetDpiForWindow(parent)/96;point=W.POINT(int(200*scale+.5),int(120*scale+.5));assert u.ClientToScreen(parent,C.byref(point))
        work=info(u.MonitorFromPoint(point,2)).work
        scale=u.GetDpiForWindow(popup)/96;width=int(260*scale+.5);height=int(150*scale+.5)
        x=max(work.left,min(point.x,work.right-width));y=max(work.top,min(point.y,work.bottom-height))
        return [x,y,x+width,y+height],[work.left,work.top,work.right,work.bottom]
    def capture(label,popup):
        picture=desktop.pixels(popup);path=out/(label+'.png');owned.png(path,picture)
        report['images'].append(dict(file=path.name,sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    try:
        run('--check',str(scene))
        with (out/'scene.log').open('w',encoding='utf-8') as log:
            process=subprocess.Popen([str(binary),'--scene',str(scene),'--no-hud','--stall','0','--seconds','180'],env=env,stdout=log,stderr=log,creationflags=flags)
            parent=until(lambda:desktop.window(process.pid,'Owned popup placement CI'))
            until(lambda:ask('get opened')=='false');ask('fact opened true')
            popup=until(lambda:desktop.window(process.pid,'pleamar surface '))
            assert u.GetWindow(popup,4)==parent,'native popup must belong to its scene window'
            def text_ready():
                width,height,pixels=desktop.pixels(popup)
                return sum(min(pixels[(y*width+x)*4:(y*width+x)*4+3])>200 for y in range(20,50) for x in range(12,225))>80
            until(text_ready)
            work=info(u.MonitorFromWindow(parent,2)).work
            scale=u.GetDpiForWindow(parent)/96
            anchor=W.POINT(int(200*scale+.5),int(120*scale+.5));assert u.ClientToScreen(parent,C.byref(anchor))
            parent_rect=rect(parent);dx=anchor.x-parent_rect[0];dy=anchor.y-parent_rect[1]
            left=work.left-dx-40;right=work.right-dx-40
            top=work.top-dy-40;bottom=work.bottom-dy-40
            moves=[('top-left',left,top),('bottom-right',right,bottom),
                ('bottom-left',left,bottom),('top-right',right,top),
                ('following',work.left+30,work.top+50)]
            for label,x,y in moves:
                assert u.SetWindowPos(parent,None,x,y,360,240,0x0010|0x0004|0x0200)
                until(lambda:rect(popup)==expected(parent,popup)[0])
                target,area=expected(parent,popup);actual=rect(popup)
                assert actual[0]>=area[0] and actual[1]>=area[1] and actual[2]<=area[2] and actual[3]<=area[3]
                if label.startswith('top-'):assert actual[1]==area[1],(label,actual,area)
                if label.startswith('bottom-'):assert actual[3]==area[3],(label,actual,area)
                if label.endswith('-left'):assert actual[0]==area[0],(label,actual,area)
                if label.endswith('-right'):assert actual[2]==area[2],(label,actual,area)
                report['placements'].append(dict(stage=label,parent=rect(parent),popup=actual,expected=target,work_area=area,dpi=u.GetDpiForWindow(popup)))
                time.sleep(.4);capture(label,popup)
            u.ShowWindow(parent,6)
            until(lambda:ask('get opened')=='false');until(lambda:not u.IsWindowVisible(popup))
            report['minimized_owner_closes_popup']=True
            u.ShowWindow(parent,9);ask('fact opened true')
            popup=until(lambda:desktop.window(process.pid,'pleamar surface '))
            until(lambda:rect(popup)==expected(parent,popup)[0]);time.sleep(.5);capture('reopened',popup)
            u.ShowWindow(parent,0)
            until(lambda:ask('get opened')=='false');until(lambda:not u.IsWindowVisible(popup))
            report['hidden_owner_closes_popup']=True
            assert u.PostMessageW(parent,0x10,0,0);assert process.wait(timeout=20)==0
        logs=(out/'scene.log').read_text(encoding='utf-8');assert 'runtime error:' not in logs and 'first frame' in logs
        report['normal_close']=True;report['passed']=True
    finally:
        if process and process.poll() is None:process.kill();process.wait(timeout=10)
        (out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()
