"""Compare native retained surfaces with complete repaint on disposable CI only."""
from pathlib import Path
import argparse, hashlib, importlib.util, json, os, re, subprocess, sys, time

spec = importlib.util.spec_from_file_location('owned_desktop', Path(__file__).with_name('windows-agent-guard-ci.py'))
owned = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owned)

SOURCE = '''scene RetainedSurfaceCI {
    surface { size: 640, 480; anchor: center; level: overlay; keyboard: none; reserve: 0; open: showing }
    fact showing = true
    fact x = 90
    fact fade = 0.65
    box { from: 8, 8; size: 624, 464; corner: 36; color: #172127; opacity: 85% }
    box { from: 32, 28; size: 580, 45; color: #172127 }
    text "Retained canvas · España 日本語" { at: 40, 35; size: 22; color: #ffffff }
    body {
        color: #9ed6bd; opacity: 60%; shadow: 3, 6, 12, 45%
        box { from: x, 100; size: 90, 70; corner: 20 }
    }
    group {
        opacity: fade
        box { from: 70, 235; size: 220, 105; corner: 28; color: #dd9060 }
        ellipse { at: 220, 285; radius: 60; color: #a0c6ee }
        text "Alpha" { at: 100, 267; size: 24; color: #ffffff }
    }
    group {
        blur: 4
        box { from: x + 270, 225; size: 75, 70; corner: 22; color: #dd6699 }
    }
}'''

def title_ready(picture):
    # The first frame can precede asynchronous font discovery by several seconds.
    # The opaque title plate prevents text behind this transparent window from
    # satisfying readiness before the fixture's own glyphs have been rendered.
    width,height,pixels=picture
    if width<610 or height<73:return False
    # A newly created transparent HWND also exposes the desktop before its first
    # frame. Require the plate itself, above the title's glyphs, before counting.
    if any(pixels[(30*width+x)*4:(30*width+x)*4+3]!=b'\x27\x21\x17' for x in range(35,610)):return False
    return sum(min(pixels[(y*width+x)*4:(y*width+x)*4+3])>200
        for y in range(35,65) for x in range(40,400))>=200

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if (sys.platform!='win32' or os.environ.get('GITHUB_ACTIONS')!='true'
        or os.environ.get('RUNNER_ENVIRONMENT')!='github-hosted' or os.environ.get('PLEAMAR_CI_RETAINED')!='1'):
        raise RuntimeError('Visible retained fixture requires the explicit step on a disposable GitHub-hosted Windows runner')
    binary=args.binary.resolve(strict=True);out=args.output.resolve();temporary=Path(os.environ['RUNNER_TEMP']).resolve()
    assert out!=temporary and out.is_relative_to(temporary)
    out.mkdir(parents=True,exist_ok=False)
    desktop=owned.Desktop();pictures={};report=dict(passed=False,environment='github-hosted',physical_input=False,images=[],comparison=[])
    flags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS
    try:
        for mode in ['full','retained','automatic']:
            folder=out/mode;folder.mkdir();scene=folder/'Canvas ñ.plm';scene.write_text(SOURCE,encoding='utf-8')
            env=dict(os.environ,PLEAMAR_SOCKET_DIR=f'retained-ci-{os.getpid()}-{mode}',PLEAMAR_NO_RELAUNCH='1',
                PLEAMAR_TEST_WINDOWS='1',PLEAMAR_TIMING='1',PLEAMAR_RETAINED_SURFACE='1')
            if mode=='full':env['PLEAMAR_FULL_REPAINT']='1'
            else:env.pop('PLEAMAR_FULL_REPAINT',None)
            if mode=='automatic':env.pop('PLEAMAR_RETAINED_SURFACE',None)
            def run(*arguments):
                result=subprocess.run([str(binary),*arguments],env=env,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=15,creationflags=flags)
                assert result.returncode==0 and not result.stdout.startswith('?'),result.stdout+result.stderr
                return result.stdout.strip()
            def ask(command):return run('--say',scene.stem,command)
            def until(predicate):
                deadline=time.monotonic()+30
                while time.monotonic()<deadline:
                    assert process.poll() is None,'scene exited unexpectedly'
                    try:
                        value=predicate()
                        if value:return value
                    except (AssertionError,OSError):pass
                    time.sleep(.1)
                raise TimeoutError('owned retained scene did not reach its expected state')
            run('--check',str(scene))
            process=None;hwnd=None
            with (folder/'scene.log').open('w',encoding='utf-8') as log:
                try:
                    process=subprocess.Popen([str(binary),'--scene',str(scene),'--no-hud','--stall','0','--seconds','120'],env=env,stdout=log,stderr=log,creationflags=flags)
                    hwnd=until(lambda:desktop.window(process.pid,'pleamar surface 0 · '))
                    until(lambda:ask('get showing')=='true')
                    until(lambda:title_ready(desktop.pixels(hwnd)))
                    for stage,x,fade in [('initial',90,.65),('moved',135,.65),('alpha',135,.45),('returned',90,.65),('reopened',135,.65),('resized',110,.65)]:
                        if stage=='reopened':
                            ask('fact showing false');time.sleep(.6);ask('fact showing true')
                        if stage=='resized':
                            scene.write_text(SOURCE.replace('size: 640, 480','size: 620, 460'),encoding='utf-8')
                            time.sleep(2)
                            hwnd=until(lambda:desktop.window(process.pid,'pleamar surface 0 · '))
                        ask(f'fact x {x}');ask(f'fact fade {fade}')
                        until(lambda:ask('get x')==str(x))
                        time.sleep(1.2)
                        picture=desktop.pixels(hwnd);pictures[(mode,stage)]=picture
                        path=folder/(stage+'.png');owned.png(path,picture)
                        report['images'].append(dict(file=str(path.relative_to(out)).replace('\\','/'),sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
                    ask('quit');assert process.wait(timeout=20)==0
                finally:
                    if process and process.poll() is None:
                        process.kill();process.wait(timeout=10)
            logs=(folder/'scene.log').read_text(encoding='utf-8')
            assert 'runtime error:' not in logs and 'first frame' in logs
            count=logs.count('retained surface allocated')
            policy=re.findall(r'retained surface policy: (\S+) · adapter: (\w+) · enabled: (true|false)',logs)
            assert policy and len(set(policy))==1,logs
            setting,adapter,selected=policy[0]
            expected=mode=='retained' or (mode=='automatic' and adapter=='Cpu')
            assert (selected=='true')==expected,(mode,policy)
            assert (count>=2 if expected else count==0),(mode,count,logs[-1500:])
            report.setdefault('policy',{})[mode]=dict(setting=setting,adapter=adapter,enabled=expected,allocations=count)
        for mode in ['retained','automatic']:
            for stage in ['initial','moved','alpha','returned','reopened','resized']:
                a,b=pictures[('full',stage)],pictures[(mode,stage)]
                assert a[:2]==b[:2]
                differences=[abs(a[2][i]-b[2][i]) for i in range(len(a[2])) if i%4!=3]
                bad=sum(v>2 for v in differences);maximum=max(differences)
                report['comparison'].append(dict(mode=mode,stage=stage,channels_over_two=bad,max_difference=maximum))
        assert all(c['channels_over_two']==0 for c in report['comparison']),report['comparison']
        report['passed']=True
    finally:
        (out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()
