"""Replay regressions against the built UI in WebKitGTK, with fake native data.

Run after npm run build:
  xvfb-run -a python3 output/tauri-redunar/tests/replay-webview.py
No production state, native commands, or saved clips are accessed.
"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import base64
import json
import subprocess
import tempfile
import threading

from webkit_fixture import configure_webkit
configure_webkit()
import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2

NATIVE = Path(__file__).resolve().parents[1]
ARTIFACTS = NATIVE.parents[1] / ".redunar-build/reports/replay-ui"
ARTIFACTS.mkdir(parents=True, exist_ok=True)


def pump(milliseconds):
    loop = GLib.MainLoop()
    GLib.timeout_add(milliseconds, lambda: (loop.quit(), False)[1])
    loop.run()


with tempfile.TemporaryDirectory(prefix="redunar-replay-ui-") as temporary:
    fixture = Path(temporary)
    subprocess.run([
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
        "testsrc2=s=640x360:r=10", "-t", "3", "-c:v", "libx264",
        "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(fixture / "fixture.mp4"),
    ], check=True)
    subprocess.run([
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-i",
        str(fixture / "fixture.mp4"), "-frames:v", "1", str(fixture / "thumb.jpg"),
    ], check=True)
    jpeg = base64.b64encode((fixture / "thumb.jpg").read_bytes()).decode()

    for label, size in [('ultrawide', '840x360'), ('classic', '480x360'), ('portrait', '360x640')]:
        subprocess.run([
            'ffmpeg', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i',
            f'testsrc2=s={size}:r=10', '-t', '3', '-c:v', 'libx264',
            '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(fixture / f'{label}.mp4'),
        ], check=True)
    media_paths = {f'/{name}.mp4': fixture / f'{name}.mp4'
                   for name in ['fixture', 'ultrawide', 'classic', 'portrait']}

    class Handler(SimpleHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            media_path = media_paths.get(self.path.split("?")[0])
            if media_path is None:
                return super().do_GET()
            body = media_path.read_bytes()
            start, end = 0, len(body) - 1
            requested = self.headers.get("Range")
            if requested:
                first, last = requested.removeprefix("bytes=").split("-", 1)
                start = int(first or 0)
                end = min(int(last) if last else end, end)
            self.send_response(206 if requested else 200)
            self.send_header("Content-Type", "video/mp4")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            if requested:
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(body)}")
            self.end_headers()
            try:
                self.wfile.write(body[start:end + 1])
            except (BrokenPipeError, ConnectionResetError):
                pass  # Selecting a different clip aborts outstanding ranges.

        def translate_path(self, path):
            if path.split("?")[0] == "/fixture.mp4":
                return str(fixture / "fixture.mp4")
            return super().translate_path(path)

    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Handler, directory=str(NATIVE / "ui/dist")))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_port}"
    manager = WebKit2.UserContentManager()
    setup = """
    window.bridge={calls:[],pending:[],thumbnails:[],hold:false,thumbnailCalls:0,errors:[]};
    window.addEventListener('error',event=>bridge.errors.push(event.message));
    window.addEventListener('unhandledrejection',event=>bridge.errors.push(String(event.reason)));
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'}},invoke:async(command,args)=>{
      if(command==='replay_clips')return Array.from({length:24},(_,i)=>({file_name:i===0?'redunar-replay-1789223687-940555037-1-with-a-very-long-local-recording-filename.mp4':`clip-${i}.mp4`,title:`Clip ${String(i).padStart(2,'0')}`,game_name:i===15?'Fixture game':null,bytes:1024,modified_unix_ns:'1000000000'}));
      if(command==='clip_playback_path'){
        bridge.calls.push(args.fileName);
        if(bridge.failPlayback)throw new Error('Fixture preparation failed');
        if(bridge.hold)return new Promise((resolve,reject)=>bridge.pending.push({resolve,reject}));
        return (bridge.mediaPath||'/fixture.mp4')+'?clip='+args.fileName;
      }
      if(command==='clip_metadata')return {duration_seconds:3,width:640,height:360,fps:30};
      if(command==='clip_thumbnail'){
        bridge.thumbnailCalls++;
        return new Promise(resolve=>bridge.thumbnails.push(()=>resolve(Array.from(atob(JPEG),c=>c.charCodeAt(0)))));
      }
      if(command==='export_replay_clip'){bridge.exported=args;if(bridge.failExport)throw Error('A clip with that filename already exists');return {file_name:args.fileName,duration_seconds:args.endSeconds-args.startSeconds,bytes:1024};}
      if(command==='replay_storage_status')return {used_bytes:24576};
      if(command==='catalog_games'||command==='session_history')return [];
      if(command==='replay_runtime_status')return {phase:'Unavailable',unavailable_reason:'OpenGL capture is unavailable for this game',completed_save_revision:0};
      if(command==='module_statuses')return {instant_replay:'Enabled'};
      if(command==='app_preferences')return {close_to_tray:false};
      if(command==='replay_preferences')return {initial_save_duration_seconds:30};
      if(command==='global_settings')throw new Error('Unused profile fixture');
      return {};
    }};
    window.check=(condition,message)=>{if(!condition)throw new Error(message)};
    """.replace("JPEG", json.dumps(jpeg))
    manager.add_script(WebKit2.UserScript.new(setup, WebKit2.UserContentInjectedFrames.TOP_FRAME, WebKit2.UserScriptInjectionTime.START, None, None))
    view = WebKit2.WebView.new_with_user_content_manager(manager)
    window = Gtk.Window()
    window.set_default_size(1400, 1000)
    window.add(view)
    window.show_all()

    def js(source):
        loop = GLib.MainLoop()
        outcome = []

        def complete(webview, result, _):
            try:
                outcome.append(webview.evaluate_javascript_finish(result).to_string())
            except Exception as error:
                outcome.append(error)
            loop.quit()

        view.evaluate_javascript(source, -1, None, None, None, complete, None)
        loop.run()
        if isinstance(outcome[0], Exception):
            raise outcome[0]
        return outcome[0]

    def screenshot(name):
        loop = GLib.MainLoop()

        def snapshot(webview, result, _):
            webview.get_snapshot_finish(result).write_to_png(str(ARTIFACTS / name))
            loop.quit()

        view.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, WebKit2.SnapshotOptions.NONE, None, snapshot, None)
        loop.run()

    try:
        view.load_uri(origin + "/#replay")
        pump(1800)
        screenshot("replay-initial.png")
        js("""
        check(document.querySelectorAll('[data-clip]').length===24,'fixture loaded');
        check(document.querySelector('.capture-status-detail').textContent.includes('OpenGL capture is unavailable for this game'),'native replay reason is visible');
        window.rail=document.querySelector('.clip-list');
        check(rail.scrollWidth<=rail.clientWidth+1,'long filename does not overflow rail');
        check(getComputedStyle(document.querySelector('.clip-select strong')).whiteSpace==='normal','filenames wrap');
        check(!document.querySelector('.clip-delete'),'deletion belongs to selected toolbar');
        check(!document.querySelector('.editor').contains(document.querySelector('#save-duration')),'save footer spans workspace');
        """)
        pump(200)
        js("""
        window.search=document.querySelector('#clip-search');
        search.value='clip-1';search.dispatchEvent(new Event('input',{bubbles:true}));
        check(document.querySelectorAll('[data-clip]').length===11,'filtered clips');
        document.querySelector('#save-duration').value='120';
        rail.scrollTop=500;window.railTop=rail.scrollTop;
        const card=document.querySelector('[data-clip="clip-15.mp4"]');
        card.focus({preventScroll:true});card.click();
        check(document.querySelector('.clip-list')===rail,'selection retained rail');
        check(rail.scrollTop===railTop&&railTop>0,'selection retained rail scroll');
        check(document.activeElement===card,'selection retained focus');
        check(search.value==='clip-1','selection retained search');
        check(document.querySelector('#save-duration').value==='120','selection retained save duration');
        check(document.querySelector('[data-selected-clip-title]').textContent==='clip-15.mp4','selected title');
        check(document.querySelector('[data-selected-clip-title]').textContent==='clip-15.mp4','filename caption');
        check(document.querySelector('.native-video-caption strong').textContent==='Fixture game','recorded game label');
        window.selectedVideo=document.querySelector('#clip-video');
        window.beforeCalls=bridge.calls.length;
        card.click();
        check(document.querySelector('#clip-video')===selectedVideo,'same clip retained player');
        check(bridge.calls.length===beforeCalls,'same clip did not prepare twice');
        bridge.thumbnails.splice(0).forEach(resolve=>resolve());
        """)
        pump(1000)
        js("""
        bridge.thumbnails.splice(0).forEach(resolve=>resolve());
        check(document.querySelector('.clip-list')===rail&&rail.scrollTop===railTop,'thumbnail retained rail scroll');
        check(document.querySelector('#clip-video')===selectedVideo,'thumbnail retained player');
        check(!document.querySelector('#clip-play-toggle').disabled,'media ready');
        window.toggleIcon=document.querySelector('#clip-play-toggle svg');
        window.exportIcon=document.querySelector('#export-clip-button svg');
        selectedVideo.dispatchEvent(new Event('timeupdate'));
        check(document.querySelector('#clip-play-toggle svg')===toggleIcon,'idle update retained play icon');
        check(document.querySelector('#export-clip-button svg')===exportIcon,'idle update retained export icon');
        """)
        # The selection made during the first thumbnail batch is retried once.
        pump(1000)
        js("bridge.thumbnails.splice(0).forEach(resolve=>resolve())")
        pump(150)
        js("""
        check(document.querySelector('[data-clip="clip-15.mp4"] img'),'selected thumbnail patched');
        check(document.querySelector('[data-clip="clip-15.mp4"] [data-clip-duration]').textContent==='00:03','real duration badge');
        check(document.querySelector('.clip-list')===rail&&rail.scrollTop===railTop,'visible image retained rail scroll');
        bridge.hold=true;bridge.calls=[];
        document.querySelector('[data-clip="clip-16.mp4"]').click();
        document.querySelector('[data-clip="clip-17.mp4"]').click();
        document.querySelector('[data-clip="clip-18.mp4"]').click();
        check(bridge.calls.join(',')==='clip-16.mp4','bounded preparation');
        check(document.querySelector('[data-selected-clip-title]').textContent==='clip-18.mp4','selection updates while native preparation pending');
        check(document.querySelector('#clip-play-toggle').disabled,'old media readiness cleared');
        bridge.pending.shift().resolve('/fixture.mp4?obsolete');
        """)
        pump(100)
        js("""
        check(bridge.calls.join(',')==='clip-16.mp4,clip-18.mp4','intermediate preparation skipped');
        check(!document.querySelector('#clip-video').hasAttribute('src'),'stale URL discarded');
        bridge.pending.shift().resolve('/fixture.mp4?current');
        """)
        pump(500)
        js("""
        check(document.querySelector('#clip-video').getAttribute('src')==='/fixture.mp4?current','newest URL loaded');
        check(rail.scrollTop===railTop,'rapid selection retained scroll');
        bridge.hold=false;
        """)

        # The window content scrolls independently from the document and rail.
        window.resize(900, 850)
        pump(300)
        screenshot("replay-narrow.png")
        js("""
        window.workspaceScroll=document.querySelector('.window-content');
        const next=document.querySelector('[data-clip="clip-19.mp4"]');
        next.scrollIntoView({block:'center'});window.pageTop=workspaceScroll.scrollTop;
        next.click();
        check(pageTop>0&&workspaceScroll.scrollTop===pageTop,'selection retained workspace scroll '+JSON.stringify({pageTop,after:workspaceScroll.scrollTop,height:workspaceScroll.scrollHeight,client:workspaceScroll.clientHeight,documentTop:scrollY}));
        """)
        pump(600)
        js("check(workspaceScroll.scrollTop===pageTop,'metadata retained workspace scroll');check(bridge.errors.length===0,bridge.errors.join(';'))")
        js("""
        bridge.hold=true;
        document.querySelector('[data-clip="clip-18.mp4"]').click();
        window.detachedVideo=document.querySelector('#clip-video');
        location.hash='overview';
        """)
        pump(100)
        js("bridge.pending.shift().reject('obsolete failure');bridge.hold=false")
        pump(100)
        js("""
        check(document.body.dataset.page==='overview','navigation works during preparation');
        check(document.querySelector('#toast').hidden,'stale preparation error stayed hidden');
        location.hash='replay';
        """)
        pump(400)
        js("""
        check(!document.querySelector('#clip-play-toggle').disabled,'return to replay loads media');
        const current=document.querySelector('#clip-video');
        const before=document.querySelector('#clip-time').textContent;
        detachedVideo.dispatchEvent(new Event('seeked'));
        check(document.querySelector('#clip-time').textContent===before,'old media event ignored');
        document.querySelector('#clip-search').value='clip-1';
        document.querySelector('#clip-search').dispatchEvent(new Event('input',{bubbles:true}));
        window.rail=document.querySelector('.clip-list');
        check(bridge.errors.length===0,bridge.errors.join(';'));
        """)
        for _ in range(30):
            if js("document.querySelector('#clip-filmstrip').dataset.frames||''") == '8':
                break
            pump(200)
        js("""
        const filmstrip=document.querySelector('#clip-filmstrip');
        check(filmstrip.dataset.frames==='8','eight bounded frame previews');
        const context=filmstrip.getContext('2d');
        const first=Array.from(context.getImageData(0,0,160,90).data);
        const last=Array.from(context.getImageData(1120,0,160,90).data);
        check(first.some((channel,index)=>channel!==last[index]),'filmstrip samples different clip moments '+JSON.stringify({first:first.slice(0,8),last:last.slice(0,8),sum:first.reduce((a,b)=>a+b,0),width:filmstrip.width,height:filmstrip.height}));
        check(document.querySelector('#clip-video').currentTime===0,'sampling did not seek player');
        const start=document.querySelector('#trim-start-bar'),end=document.querySelector('#trim-end-bar');
        start.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));
        end.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));
        check(start.getAttribute('aria-valuenow')==='0.25','keyboard trim start');
        check(end.getAttribute('aria-valuenow')==='2.75','keyboard trim end');
        check(document.querySelector('#clip-selected-duration').textContent==='00:02.5','selected duration follows trim');
        check(document.querySelector('#trim-start-value').textContent==='00:00.3','visible In follows trim');
        check(document.querySelector('#clip-filmstrip')===filmstrip,'trimming retained filmstrip');
        document.querySelector('#export-clip-button').click();
        const form=document.querySelector('#trim-export-form');
        check(form.elements.start.value==='0.25'&&form.elements.end.value==='2.75','exact export boundaries');
        form.elements.outputName.value='My named trim';bridge.failExport=true;form.requestSubmit();
        """)
        pump(350)
        js("check(document.querySelector('#dialog').open,'export collision retains dialog');check(document.querySelector('#dialog-body').textContent.includes('already exists'),'collision explanation');check(document.querySelector('[name=outputName]').value==='My named trim','filename retained for retry');bridge.failExport=false;document.querySelector('#trim-export-form').requestSubmit();")
        pump(350)
        js("check(bridge.exported.startSeconds===.25&&bridge.exported.endSeconds===2.75,'export sent exact range');check(bridge.exported.outputName==='My named trim','filename stem sent');check(!('destination' in bridge.exported),'native owns configured folder');check(document.querySelector('[data-action=show-export-folder]')&&document.querySelector('[data-action=open-exported-clip]'),'completion actions')")
        pump(1200)
        js("""
        document.querySelector('#trim-start-bar').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',shiftKey:true,bubbles:true}));
        check(Number(document.querySelector('#trim-start-bar').getAttribute('aria-valuenow'))<=2.9,'trim handles cannot cross');
        document.querySelector('#trim-start-bar').dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true}));
        document.querySelector('#trim-end-bar').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));
        check(bridge.errors.length===0,bridge.errors.join(';'));
        """)
        js("""
        check(document.querySelector('#clip-video').preload==='auto','normal video preloading');
        window.retryRail=document.querySelector('.clip-list');
        retryRail.scrollTop=400;window.retryTop=retryRail.scrollTop;
        document.querySelector('#clip-video').dispatchEvent(new Event('error'));
        check(!document.querySelector('#retry-clip').hidden,'media failure offers retry');
        check(document.querySelector('#clip-play-toggle').disabled,'failed player is disabled');
        check(!document.querySelector('#clip-video').hasAttribute('src'),'failed stream released');
        document.querySelector('#media-state-overlay').scrollIntoView({block:'center'});
        """)
        screenshot("replay-load-failure.png")
        js("""
        bridge.failPlayback=true;document.querySelector('#retry-clip').click();
        """)
        pump(100)
        js("""
        check(document.querySelector('#media-state-detail').textContent==='Fixture preparation failed','preparation failure shown');
        check(!document.querySelector('#retry-clip').hidden,'preparation failure remains retryable');
        bridge.failPlayback=false;document.querySelector('#retry-clip').click();
        """)
        pump(400)
        js("""
        check(document.querySelector('#retry-clip').hidden,'successful retry clears failure');
        check(!document.querySelector('#clip-play-toggle').disabled,'retry loads playable video');
        check(document.querySelector('.clip-list')===retryRail&&retryRail.scrollTop===retryTop,'retry retains rail and scroll');
        """)
        window.resize(1400, 1000)
        pump(300)
        js("scrollTo(0,150);document.querySelector('.clip-list').scrollTop=500")
        pump(200)
        screenshot("replay-webview.png")
        js("document.querySelector('#export-clip-button').click()")
        pump(100)
        screenshot("export-dialog.png")
        js("check(document.querySelector('[name=outputName]'),'editable export filename');check(document.querySelector('#export-destination'),'configured destination');check(!document.querySelector('.export-summary'),'trim details stay in editor');document.querySelector('#dialog [data-close]').click();const seek=document.querySelector('#player-seek');check(!seek.disabled,'seek enabled');seek.value='1';seek.dispatchEvent(new Event('input',{bubbles:true}));")
        pump(200)
        js("check(Math.abs(document.querySelector('#clip-video').currentTime-1)<.1,'precision seek reaches requested time')")
        js("const selected=document.querySelector('[data-selected-clip-title]').textContent;document.querySelector('[data-action=delete-selected-clip]').click();check(JSON.parse(document.querySelector('#trash-clips-form [name=fileNames]').value)[0]===selected,'delete targets current selection');document.querySelector('#dialog [data-close]').click();")
        js("const resetStartHandle=document.querySelector('#trim-start-bar');resetStartHandle.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));check(!document.querySelector('[data-action=reset-trim]').disabled,'trim reset available');document.querySelector('[data-action=reset-trim]').click();check(document.querySelector('#trim-start-value').textContent==='00:00.0','trim starts at zero after reset');check(document.querySelector('#clip-selected-duration').textContent==='00:03.0','reset retains entire recording');check(!document.querySelector('#center-play').hidden,'center play visible while paused');document.querySelector('#center-play').click();")
        pump(300)
        js("check(!document.querySelector('#clip-video').paused,'center play starts video');check(document.querySelector('#center-play').hidden,'center control leaves playing picture clear');document.querySelector('#clip-play-toggle').click();")
        # Exercise actual decoded display dimensions, including a landscape
        # thumbnail on non-landscape recordings, rather than mocking videoWidth.
        for label, width, height in [('fixture', 640, 360), ('ultrawide', 840, 360), ('classic', 480, 360), ('portrait', 360, 640)]:
            js(f"bridge.mediaPath='/{label}.mp4';document.querySelector('[data-action=retry-clip]').click();")
            pump(700)
            for viewport in [1400, 640]:
                window.resize(viewport, 1000)
                pump(250)
                js(f"""
                {{
                const video=document.querySelector('#clip-video'),stage=video.parentElement;
                const rect=video.getBoundingClientRect(),box=stage.getBoundingClientRect();
                check(video.videoWidth==={width}&&video.videoHeight==={height},'decoded {label} dimensions');
                check(Math.abs(rect.height-rect.width*{height}/{width})<1,'native {label} aspect at {viewport}');
                check(Math.abs(box.height-rect.height)<1&&Math.abs(box.width-rect.width)<1,'video fills stage');
                check(getComputedStyle(video).objectFit==='contain','whole frame preserved');
                check(parseFloat(getComputedStyle(stage).borderTopLeftRadius)>0&&getComputedStyle(stage).overflow==='hidden','rounded image corners');
                check(Math.abs(document.querySelector('.player-controls').getBoundingClientRect().top-box.bottom)<2,'controls directly below clip');
                check(document.documentElement.scrollWidth<=innerWidth+1,'no horizontal overflow');
                }}
                """)
                if viewport==1400:
                    js("scrollTo(0,150)")
                    screenshot(f'replay-aspect-{label}.png')
        window.resize(1400,1000);pump(200)
        js("bridge.mediaPath='/ultrawide.mp4';window.aspectViewport=document.querySelector('.window-content');aspectViewport.scrollTop=100;window.aspectScroll=aspectViewport.scrollTop;window.aspectRail=document.querySelector('.clip-list');aspectRail.scrollTop=400;window.aspectRailTop=aspectRail.scrollTop;document.querySelector('[data-clip=\"clip-15.mp4\"]').focus({preventScroll:true});document.querySelector('[data-clip=\"clip-15.mp4\"]').click();")
        pump(700)
        js("check(aspectViewport.scrollTop===Math.min(aspectScroll,aspectViewport.scrollHeight-aspectViewport.clientHeight),'aspect change retains workspace scroll within available content');check(document.querySelector('.clip-list')===aspectRail&&aspectRail.scrollTop===aspectRailTop,'aspect change retains rail scroll');check(bridge.errors.length===0,bridge.errors.join(';')); ")
        print('PASS: WebKitGTK rail/document scroll, filter, focus, save duration, thumbnail updates, media readiness, unchanged control icons, rapid selection, eight real frame samples, keyboard trimming, and export boundaries.')
    finally:
        window.destroy()
        server.shutdown()
