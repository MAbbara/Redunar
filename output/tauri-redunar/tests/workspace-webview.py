"""Exercise the built production UI in WebKitGTK with isolated command fixtures.
No host profile, real game, user clip, tray or hardware settings are changed.
"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import json
import re
import threading
from webkit_fixture import configure_webkit
configure_webkit()
import gi
gi.require_version('Gtk','3.0')
gi.require_version('WebKit2','4.1')
from gi.repository import GLib, Gtk, Gdk, GdkPixbuf, WebKit2
NATIVE=Path(__file__).resolve().parents[1]
ARTIFACTS=NATIVE.parents[1]/'target/ui-review'
ARTIFACTS.mkdir(parents=True,exist_ok=True)
class Handler(SimpleHTTPRequestHandler):
    def log_message(self,*_): pass
server=ThreadingHTTPServer(('127.0.0.1',0),partial(Handler,directory=str(NATIVE/'ui/dist')))
threading.Thread(target=server.serve_forever,daemon=True).start()
manager=WebKit2.UserContentManager()
fixture=r"""
window.bridge={calls:[],errors:[],conflict:false,unavailable:false,automaticUpdates:true,startupUpdateAvailable:true,revision:1,modules:{frame_metrics:'Enabled',in_game_overlay:'Enabled',instant_replay:'Enabled'}};
window.addEventListener('error',e=>bridge.errors.push(e.message));
window.addEventListener('unhandledrejection',e=>bridge.errors.push(String(e.reason)));
function artworkFixture(width,height,label,mime='image/png'){const canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;const ctx=canvas.getContext('2d');const gradient=ctx.createLinearGradient(0,0,width,height);gradient.addColorStop(0,'#8b202a');gradient.addColorStop(1,'#171717');ctx.fillStyle=gradient;ctx.fillRect(0,0,width,height);ctx.fillStyle='#ffffff';ctx.font='32px sans-serif';ctx.fillText(label,30,70);return Array.from(atob(canvas.toDataURL(mime).split(',')[1]),c=>c.charCodeAt(0));}
const posterFixture=artworkFixture(300,450,'Poster');
const bannerFixture=artworkFixture(1200,350,'Landscape banner');
const captureFixture=artworkFixture(640,360,'Capture fixture','image/jpeg');
const values={overlay:true,preset:'Compact',layout:'Grid',palette:'Redunar',branding:true,position:'Top left',scale:100,opacity:90,metrics:['FPS'],captureMetrics:true,replayEnabled:true,fps:60,quality:'Balanced',format:'MKV',shortcuts:{overlay:'Ctrl+Shift+R',30:'Ctrl+Shift+S'}};
function replayPreferencesFixture(){return {microphone_mode:bridge.microphone||'off',microphone_source:null,initial_save_duration_seconds:30,resolved_directory:'/fixture/Videos/Redunar Replays',custom_save_parent:'/fixture',close_overlay_on_outside_click:bridge.outsideClick!==false};}
const games=[{id:'1',name:'Test game with a long local catalog title',executable:'/fixture/game',arguments:['--one','--two'],working_directory:null,steam_app_id:42,revision:'1',launch_revision:'launch-1',overrides:{}},{id:'2',name:'Second local game',executable:'/fixture/second',arguments:[],revision:'1',launch_revision:'launch-2',overrides:{overlay:false}}];
window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'}},invoke:async(command,args={})=>{
 bridge.calls.push({command,args:structuredClone(args)});
 if(command==='save_replay'&&bridge.failSave)throw Error('Fixture save failed');
 if(command==='replay_runtime_status'&&bridge.failRuntime)throw Error('Fixture runtime unavailable');
 if(command==='daemon_status'&&bridge.failDaemon)throw Error('Fixture native status unavailable');
 if(command==='daemon_status')return bridge.unavailable?{available:false}:{available:true,diagnostic_log_status:bridge.logStatus||'off',revision:++bridge.revision,cpu_model:'Fixture CPU',cpu_utilization_percent:32,cpu_temperature_celsius:61,gpu_model:'Fixture GPU',gpu_utilization_percent:81,gpu_temperature_celsius:67,ram_used_bytes:12884901888,ram_total_bytes:34359738368,vram_used_bytes:5368709120,vram_total_bytes:12884901888};
 if(command==='session_status'&&bridge.idle)return {phase:'Idle',can_end:false,launch_locked:false,history_revision:1};
 if(command==='session_status')return {phase:'Running',game:games[0].name,can_end:true,launch_locked:true,elapsed_seconds:1938,captures_saved:3,profile_label:'Session profile',feature_summary:'Metrics enabled · replay enabled',restoration:'NotRequired',history_revision:1,measurements:{revision:bridge.revision,average_fps:141,frame_time_ms:7.1,one_percent_low_fps:118,point_one_percent_low_fps:97,frame_intervals_ns:[7100000+Math.round(Math.sin(bridge.revision)*700000)],phase:'Live'}};
 if(command==='replay_runtime_status'&&bridge.replayUnavailable)return {phase:'Unavailable',can_save:false,buffered_seconds:0,unavailable_reason:fixtureResolutionRejection};
 if(command==='replay_runtime_status'&&bridge.replayFailure)return {phase:'Failed',can_save:false,buffered_seconds:0,failure:'The video encoder failed. Check Replay hardware support, then relaunch the game.'};
 if(command==='replay_runtime_status'&&bridge.replayWaiting)return {phase:'Inactive',can_save:false,buffered_seconds:0,pending_reason:'Waiting for recordable game frames. Game startup or a display change can pause Replay.'};
 if(command==='replay_runtime_status'&&bridge.replayInactive)return {phase:'Inactive',can_save:false,buffered_seconds:0};
 if(command==='replay_runtime_status')return {phase:'Buffering',can_save:true,buffered_seconds:30,received_frame_count:600,encoded_packet_count:600,audio_packet_count:1500,audio_byte_count:4500,audio_active:true,completed_save_revision:1};
 if(command==='module_statuses')return structuredClone(bridge.modules);
 if(command==='discover_games')return structuredClone(bridge.discovered||[]);
 if(command==='import_discovered_games')return structuredClone(games);
 if(command==='catalog_games')return structuredClone(games);
 if(command==='game_installation_statuses'){if(bridge.installationFailure)throw Error('Fixture library unavailable');return bridge.installation||{'1':'installed','2':'installed'};}
 if(command==='game_poster')return posterFixture;
 if(command==='game_banner')return args.gameId==='1'?bannerFixture:null;
 if(command==='replay_clips')return Array.from({length:3},(_,i)=>({file_name:`redunar-replay-1789254367-629872093-${i+1}-a-long-capture-filename.mkv`,title:`Capture ${i+1}`,game_name:i===0?games[0].name:null,bytes:68000000,modified_unix_ns:'1789300000000000000',duration_seconds:30}));
 if(command==='clip_metadata')return {duration_seconds:30,width:640,height:360,fps:60};
 if(command==='clip_thumbnail')return captureFixture;
 if(command==='replay_storage_status')return {used_bytes:0};
 if(command==='app_preferences')return {close_to_tray:false,automatic_updates:bridge.automaticUpdates,diagnostic_log:bridge.logRequested===true,diagnostic_log_status:bridge.logStatus||'off'};
 if(command==='set_close_to_tray')return {close_to_tray:args.enabled,automatic_updates:bridge.automaticUpdates};
 if(command==='set_diagnostic_log'){bridge.logRequested=args.enabled;return {diagnostic_log:args.enabled,diagnostic_log_status:bridge.logStatus||'off'};}
 if(command==='set_automatic_updates'){bridge.automaticUpdates=args.enabled;return {close_to_tray:false,automatic_updates:bridge.automaticUpdates};}
 if(command==='check_for_updates'){
  if(bridge.installedUpdate)return {current_version:'0.1.2',state:'restart-needed',message:'Redunar 0.1.3 is installed. Fully quit Redunar, including its tray process, then reopen it to load the update.',latest_version:'0.1.3',package_kind:'rpm',asset_name:'redunar-app-linux-x86_64.rpm'};
  if(bridge.startupUpdateAvailable){bridge.startupUpdateAvailable=false;return {current_version:'0.1.2',state:'available',message:'Redunar 0.1.3 has a verified update package.',latest_version:'0.1.3',package_kind:'rpm',asset_name:'redunar-app-linux-x86_64.rpm'};}
  return {current_version:'0.1.3',state:'not-configured',message:'Signed update checking is not configured in this build yet.'};
 }
 if(command==='install_update')return {state:'handoff',message:'The installer finished, but Redunar could not confirm the new package version. Check again or retry installation.'};
 if(command==='global_settings')return {values:structuredClone(values),revision:'1'};
 if(command==='save_global_settings'){if(bridge.conflict)throw new Error('Saved defaults changed elsewhere. Reload before saving.');Object.assign(values,args.input);return {values:structuredClone(values),revision:'2'};}
 if(command==='save_game_profile'){games.find(g=>g.id===args.gameId).overrides=Object.fromEntries(Object.entries(args.values).filter(([,v])=>v!==null));return {games:structuredClone(games),liveNotice:'Game settings saved. The running game overlay was updated.'};}
 if(command==='update_game_launch'){bridge.launchArgs=args.arguments;return structuredClone(games);}
 if(command==='save_shortcuts'){values.shortcuts=args.shortcuts;return {values:structuredClone(values)};}
 if(command==='set_replay_overlay_behavior'){bridge.outsideClick=args.closeOnOutsideClick;return {...replayPreferencesFixture(),overlay_shortcut:values.shortcuts.overlay||''};}
 if(command==='steam_setup_status'){if(bridge.failSteamSetup)throw Error('Fixture Steam setup unavailable');return bridge.steamSetup||{available:true,configured:false,app_id:42,configuration_state:'not-configured',status:'Not configured',launch_options:'/usr/bin/redunar-steam-launch --app-id 42 -- %command%'};}
 if(command==='set_replay_microphone'){await new Promise(resolve=>setTimeout(resolve,80));if(bridge.failMicrophone)throw Error('Fixture microphone write failed');bridge.microphone=args.mode;return replayPreferencesFixture();}
 if(command==='replay_preferences')return replayPreferencesFixture();
 if(command==='replay_display_capability')return {compatible_120_modes:0,status:'No compatible display'};
 if(command==='activate_shortcuts')bridge.shortcutUnavailable=false;
 if(command==='shortcut_status'&&bridge.shortcutUnavailable)return {state:'Unavailable',message:'No readable keyboard input device'};
 if(command==='shortcut_status'||command==='activate_shortcuts')return {state:Object.values(values.shortcuts).some(Boolean)?'Active':'Inactive'};
 if(command==='session_history')return bridge.historyFixture??[0,1,2].map((n)=>({id:n+1,game:n===2?'Other local game':games[0].name,started_unix:1789300000-n*86400,duration_seconds:1800,average_fps:141-n,one_percent_low_fps:118,point_one_percent_low_fps:97,frame_intervals_ns:[7100000,7300000],timeline:Array.from({length:96},(_,i)=>({elapsed_seconds:i*1800/95,fps:135+Math.sin(i)*6,frame_time_ms:7.1+Math.sin(i)*.3,cpu_temperature_celsius:61,gpu_temperature_celsius:67,cpu_utilization_percent:32,gpu_utilization_percent:81}))}));
 return {};
}};
window.check=(condition,message)=>{if(!condition)throw new Error(message)};
window.q=s=>document.querySelector(s);
window.click=s=>{const el=q(s);check(!!el,'Missing '+s);check(!el.disabled,'Disabled '+s);el.click();};
"""
resolution_copy = re.search(r'Some\(ReplaySourceRejection::DimensionsUnsupported\) =>\s*(?:\{\s*)?"([^"]+)"', (NATIVE/'src-tauri/src/runtime.rs').read_text()).group(1)
fixture = "const fixtureResolutionRejection=" + json.dumps(resolution_copy) + ";" + fixture
fixture = "window.fixturePalettes=" + (NATIVE / "ui/overlay-palettes.json").read_text() + ";" + fixture
manager.add_script(WebKit2.UserScript.new(fixture,WebKit2.UserContentInjectedFrames.TOP_FRAME,WebKit2.UserScriptInjectionTime.START,None,None))
view=WebKit2.WebView.new_with_user_content_manager(manager)
view.set_background_color(Gdk.RGBA(0,0,0,0))
window=Gtk.Window();window.set_default_size(1440,1000);window.add(view);window.show_all()
def pump(ms=250):
    loop=GLib.MainLoop();GLib.timeout_add(ms,lambda:(loop.quit(),False)[1]);loop.run()
def js(source):
    loop=GLib.MainLoop();result=[]
    def done(w,value,_):
        try: result.append(w.evaluate_javascript_finish(value).to_string())
        except Exception as error: result.append(error)
        loop.quit()
    view.evaluate_javascript('(()=>{'+source+'})()',-1,None,None,None,done,None);loop.run()
    if isinstance(result[0],Exception):raise result[0]
    return result[0]
def snap(name):
    loop=GLib.MainLoop()
    def done(w,value,_):w.get_snapshot_finish(value).write_to_png(str(ARTIFACTS/(name+'.png')));loop.quit()
    view.get_snapshot(WebKit2.SnapshotRegion.VISIBLE,WebKit2.SnapshotOptions.NONE,None,done,None);loop.run()
def route(name):js("location.hash="+json.dumps(name));pump(400)
def wait_for(expression, timeout_ms=5000):
    for _ in range(timeout_ms//100):
        if js('return Boolean('+expression+')')=='true': return
        pump(100)
    raise AssertionError('Timed out waiting for '+expression)
checks=[]
def test(name,source,wait_ms=0):
    js(source);checks.append(name)
    if wait_ms: pump(wait_ms)
try:
    view.load_uri(f'http://127.0.0.1:{server.server_port}/#overview');pump(1800)
    test('Startup update notice directs the user to Settings',"check(q('#toast').textContent==='A Redunar update is available. Open Settings to install it.','clear startup update action')")
    test('Real overview values and compact session footer',"check(q('#ram-used').textContent==='12.0 GiB','RAM');check(q('#session-timer').textContent==='32:18','timer');check(q('#session-captures').textContent==='3 captures saved','captures');check(q('.session-end').contains(q('[data-action=end-session]')),'timer adjacent to end');window.plot=q('#live-chart');click('[data-overview-view=fps]');check(q('#live-chart')===plot,'graph toggle does not remount page');check(q('#overview-chart-label').textContent==='Frame rate','FPS selector');")
    snap('overview')
    route('replay')
    js('bridge.replayUnavailable=true');wait_for("q('#replay-phase')?.textContent===fixtureResolutionRejection")
    test('Resolution rejection explains game resolution and recording rate',"check(q('#replay-phase').textContent.includes(\"Lower the game's resolution or recording frame rate\"),'actionable resolution/rate');check(!q('#replay-phase').textContent.includes('window size'),'fullscreen-independent copy');check(document.documentElement.scrollWidth<=innerWidth,'no page overflow');")
    snap('ultrawide-resolution-rejection')
    js('bridge.replayUnavailable=false');wait_for("q('#replay-status-title').textContent==='Ready to save'")
    test('Save duration changes keep controls beside the shortcut at wide widths',"window.saveRow=q('.replay-save-row');window.saveButton=q('[data-action=save-replay]');window.saveSelect=q('#save-duration');window.saveButtonX=saveButton.getBoundingClientRect().left;window.saveSelectX=saveSelect.nextElementSibling.getBoundingClientRect().left;for(const option of saveSelect.options){saveSelect.value=option.value;saveSelect.dispatchEvent(new Event('change',{bubbles:true}));check(q('.replay-save-row')===saveRow&&q('[data-action=save-replay]')===saveButton,'controls stay mounted');check(Math.abs(saveButton.getBoundingClientRect().left-saveButtonX)<1,'save position stable');check(Math.abs(saveSelect.nextElementSibling.getBoundingClientRect().left-saveSelectX)<1,'duration position stable');check(q('#save-replay-shortcut').hidden===(option.value!=='30'),'only assigned duration shows shortcut');}saveSelect.value='30';saveSelect.dispatchEvent(new Event('change',{bubbles:true}));")
    js("q('#toast').hidden=true;q('.replay-save-row').scrollIntoView({block:'center'})");pump();snap('replay-save-assigned')
    js("q('#save-duration').value='120';q('#save-duration').dispatchEvent(new Event('change',{bubbles:true}))");snap('replay-save-unassigned')
    window.resize(640,950);pump()
    test('Compact save controls stay inside the card across durations',"const row=q('.replay-save-row').getBoundingClientRect(),button=q('[data-action=save-replay]'),select=q('#save-duration');const before=button.getBoundingClientRect();for(const option of select.options){select.value=option.value;select.dispatchEvent(new Event('change',{bubbles:true}));const rect=button.getBoundingClientRect();check(Math.abs(rect.left-before.left)<1&&Math.abs(rect.top-before.top)<1,'compact controls do not move');check(rect.left>=row.left&&rect.right<=row.right,'save button fits');check(q('.replay-shortcut-slot').getBoundingClientRect().right<=row.right,'shortcut fits');}check(document.documentElement.scrollWidth<=innerWidth,'no horizontal overflow');q('.replay-save-row').scrollIntoView({block:'center'});")
    pump();snap('replay-save-compact')
    window.resize(1440,1000);pump()
    js("bridge.replayWaiting=true");wait_for("q('#replay-status-title').textContent==='Waiting for Replay frames'")
    test('Waiting frame source uses pending styling and disables save',"check(q('#replay-status-dot').dataset.status==='pending','not a hardware error');check(q('[data-action=save-replay]').disabled,'no save without frames');")
    js("q('.window-content').scrollTo(0,0)");pump();snap('replay-waiting')
    js('bridge.replayFailure=true');wait_for("q('#replay-status-dot').dataset.status==='error'")
    test('Encoder failure remains visible over pending status',"check(q('#replay-phase').textContent.includes('video encoder failed'),'encoder error preserved');check(q('[data-action=save-replay]').disabled,'failed encoder cannot save');")
    snap('replay-encoder-failure')
    js('bridge.replayFailure=false;bridge.replayWaiting=false');wait_for("q('#replay-status-title').textContent==='Ready to save'")
    route('overview')
    test('Overview uses settings for visibility and points to Steam setup',"check(!q('[data-action=session-overlay]'),'no duplicate live button');check(q('#steam-capture-guide a').getAttribute('href')==='#library','setup link');check(q('#steam-capture-guide').hidden,'guide hidden during session');")
    js('bridge.idle=true');wait_for("!q('#steam-capture-guide').hidden")
    test('Idle Overview makes Steam capture setup discoverable',"check(q('#steam-capture-guide').textContent.includes('Set up each game once'),'setup explanation');check(q('#steam-capture-guide a').textContent==='Set up Steam capture','setup action');")
    snap('steam-play-overview')
    js('bridge.idle=false');wait_for("q('#steam-capture-guide').hidden")
    route('library');pump(300)
    test('Library uses independent landscape and portrait artwork',"const banner=q('.game-cover-banner img'),poster=q('.library-game img');check(banner?.naturalWidth===1200&&banner.naturalHeight===350,'wide banner received');check(poster?.naturalWidth===300&&poster.naturalHeight===450,'portrait retained in catalog');check(banner.src!==poster.src,'separate artwork caches');check(getComputedStyle(q('.game-cover-banner')).position==='absolute','banner is outside layout flow');const card=q('.game-cover').getBoundingClientRect();check(card.height<260,'image does not expand header');check(Math.abs(banner.getBoundingClientRect().width-card.width)<2,'banner spans header');")
    test('Library inheritance and readiness use the approved layout',"check(q('.inheritance-banner strong').textContent==='Uses global settings','inheritance heading');check(q('.inheritance-banner p').textContent.includes('apply to this game'),'inheritance explanation');check(document.querySelectorAll('[data-readiness-status]').length===3,'inline readiness');check(q('[data-readiness-status]').dataset.ready==='true','observed ready dot');check(!q('.override-control .pill'),'no duplicated inherited badge');check(q('.game-save-bar').hidden,'clean library has no action bar');check(!q('.detail-body .game-save-bar'),'bar outside settings panel');check(q('.game-profile-footer').getBoundingClientRect().bottom+20<=q('.game-profile-footer+.small-note').getBoundingClientRect().top,'note spacing');")
    test('Library drafts use viewport actions and discard hides them',"const select=q('[data-override=overlay]');select.value='Off';select.dispatchEvent(new Event('change',{bubbles:true}));const bar=q('.game-save-bar');check(!bar.hidden,'dirty library bar visible');check(getComputedStyle(bar).position==='fixed','viewport bar');check(Math.abs(innerHeight-bar.getBoundingClientRect().bottom-16)<1,'bottom aligned');check(bar.getBoundingClientRect().left>=q('.sidebar').getBoundingClientRect().right,'sidebar stays usable');click('[data-action=discard-game]');check(q('.game-save-bar').hidden,'discard hides bar');")
    snap('library-banner')
    test('Compact Steam setup is visible beside game settings without a launch or modal',"check(q('[data-library-tab=profile]').classList.contains('active'),'default game tab');check(!q('.steam-setup-status')&&!q('[data-action=check-steam-setup]'),'no routine status or retry');check(q('.steam-setup-card').getBoundingClientRect().bottom<=q('.tabbar').getBoundingClientRect().top+1,'setup above tabs');check(q('.steam-setup-card').textContent.includes('Paste this into Steam’s Launch Options, then press Play.'),'paste instruction');check(q('.steam-setup-caption').textContent==='Set up once. Redunar starts automatically with the game.','background instruction');check(q('#steam-launch-options').value==='/usr/bin/redunar-steam-launch --app-id 42 -- %command%','native option');check(q('#steam-launch-options').readOnly&&q('#steam-launch-options').getAttribute('aria-label')==='Steam Launch Options','accessible readonly option');check(q('[data-copy-steam-options]').textContent==='Copy','short action');check(!q('#dialog').open,'no setup modal');")
    js("q('#toast').hidden=true");snap('steam-play-setup')
    test('Steam option copy preserves the exact native value',"Object.defineProperty(navigator,'clipboard',{value:{writeText:async value=>{bridge.copied=value}},configurable:true});click('[data-copy-steam-options]');")
    pump()
    test('Steam copy completed',"check(bridge.copied===q('#steam-launch-options').value,'literal copy');check(q('#toast').textContent==='Steam launch options copied.','copy feedback');click('[data-library-tab=match]');")
    pump()
    test('Steam setup remains visible on Launch matching',"check(document.querySelectorAll('.steam-setup-card').length===1,'one setup panel');check(q('#steam-launch-options'),'option still visible');click('[data-game=\"2\"]');")
    pump()
    test('Direct games omit Steam setup',"check(!q('.steam-setup-card'),'no Steam panel on direct game');click('[data-game=\"1\"]');click('[data-library-tab=profile]');")
    pump()
    js("const select=q('[data-override=overlay]');select.value='Off';select.dispatchEvent(new Event('change',{bubbles:true}))")
    wait_for("q('#steam-launch-options')")
    js("window.steamField=q('#steam-launch-options');steamField.focus();steamField.setSelectionRange(10,20);window.dispatchEvent(new Event('focus'))");pump()
    test('Unchanged setup focus refresh preserves selection and profile drafts',"check(q('#steam-launch-options')===steamField&&document.activeElement===steamField,'same focused field');check(steamField.selectionStart===10&&steamField.selectionEnd===20,'selection preserved');check(q('[data-override=overlay]').value==='Off'&&!q('.game-save-bar').hidden,'draft preserved');")
    js("bridge.steamSetup={available:true,configured:true,app_id:42,configuration_state:'configured',status:'Configured',launch_options:'/usr/bin/redunar-steam-launch --app-id 42 -- %command%'};window.dispatchEvent(new Event('focus'))");pump()
    test('Configured Steam games retain their copyable option and unrelated drafts',"check(q('.steam-setup-status').textContent==='Launch option configured','verified setup');check(q('.steam-setup-card').textContent.includes('Press Play in Steam. Redunar starts automatically.'),'launch instruction');check(q('#steam-launch-options')&&!q('[data-action=check-steam-setup]'),'configured option retained');check(q('[data-override=overlay]').value==='Off'&&!q('.game-save-bar').hidden,'draft preserved after changed evidence');click('[data-action=discard-game]');")
    snap('steam-play-configured')
    js("bridge.steamSetup.configuration_state='steam-running';bridge.steamSetup.configured=false;bridge.steamSetup.status='Needs attention: Steam is running, so its in-memory Launch Options may differ from the saved file';window.dispatchEvent(new Event('focus'))");pump()
    test('Running Steam keeps compact setup without claiming setup success',"const text=q('.steam-setup-card').textContent;check(!q('.steam-setup-status')&&!q('[data-action=check-steam-setup]'),'no routine status or retry');check(text.includes('Paste this into Steam’s Launch Options, then press Play.'),'clear next step');check(!text.includes('Needs attention')&&!text.includes('in-memory')&&!text.includes('before it is saved')&&!text.includes('configured'),'no technical warning or false confirmation');check(q('#steam-launch-options'),'actionable value retained');")
    snap('steam-play-running')
    js("bridge.steamSetup.configuration_state='needs-attention';bridge.steamSetup.status='Steam user accounts do not agree on this game’s Launch Options';window.dispatchEvent(new Event('focus'))");pump()
    test('Other ambiguous Steam reasons remain visible and retryable',"check(q('.steam-setup-status').textContent==='Setup unconfirmed','other ambiguity');check(q('.steam-setup-card').textContent.includes('Steam user accounts do not agree'),'other explanation retained');check(!q('[data-action=check-steam-setup]').disabled,'ambiguity retry available');")
    js("bridge.steamSetup.available=false;bridge.steamSetup.status='Wrapper missing';click('[data-action=check-steam-setup]')");pump()
    test('Unavailable Steam setup has no unusable copy action',"check(q('.steam-setup-card').textContent.includes('Wrapper missing'),'unavailable reason');check(!q('[data-copy-steam-options]'),'no copy');check(!q('[data-action=check-steam-setup]').disabled,'unavailable retry available');")
    js("bridge.failSteamSetup=true;click('[data-action=check-steam-setup]')");pump()
    test('Steam setup errors retain a retry action',"check(q('.steam-setup-card').textContent.includes('Fixture Steam setup unavailable'),'check error');check(!q('[data-action=check-steam-setup]').disabled,'retry available');")
    js("bridge.failSteamSetup=false;delete bridge.steamSetup;click('[data-action=check-steam-setup]')");pump()
    window.resize(640,950);pump()
    test('Steam launch option fits a compact workspace after retry recovery',"const card=q('.steam-setup-card').getBoundingClientRect(),field=q('#steam-launch-options').getBoundingClientRect(),copy=q('[data-copy-steam-options]').getBoundingClientRect();check(field.left>=card.left&&field.right<=card.right,'field fits');check(copy.left>=card.left&&copy.right<=card.right,'copy fits');check(document.documentElement.scrollWidth<=innerWidth,'no page overflow');check(!q('.steam-setup-status')&&!q('[data-action=check-steam-setup]'),'retry disappears after success');q('#steam-launch-options').focus();check(document.activeElement===q('#steam-launch-options'),'keyboard selection available');")
    js("q('#toast').hidden=true;q('.steam-setup-card').scrollIntoView({block:'start'})");pump();snap('steam-play-compact')
    window.resize(1440,1000);pump()
    js("click('[data-action=scan-games]')");pump()
    test('Discovery empty state and dialog initial focus',"check(q('#dialog-title').textContent==='Scan installed games','installed title');check(q('#dialog').textContent.includes('No installed games were found.'),'installed empty message');check(q('#discovery-form [type=submit]').disabled,'empty import disabled');check(!bridge.calls.some(c=>c.command==='running_game_candidates'),'no process discovery request');check(document.activeElement===q('#dialog-title'),'focus starts at title');check(!q('#dialog [data-close]').matches(':focus-visible'),'close not highlighted');click('#dialog [data-close]');")
    js("bridge.discovered=[{candidate_id:'steam:42',name:'Already saved game',source:'Steam',source_id:42,install_directory:'/fixture/steam/existing',launch_executable:'/usr/bin/steam',importable:true},{candidate_id:'steam:730',name:'Installed game',source:'Steam',source_id:730,install_directory:'/fixture/steam/new',launch_executable:'/usr/bin/steam',importable:true},{candidate_id:'desktop:fixture',name:'Unavailable launcher',source:'Desktop entry',install_directory:'/fixture/missing',importable:false}];click('[data-action=scan-games]')");pump()
    test('Installed discovery excludes wrappers and retains import validation',"const boxes=[...document.querySelectorAll('#discovery-form input')];check(boxes.length===3,'installed candidates only');check(boxes[0].disabled&&!boxes[0].checked,'existing game excluded');check(!boxes[1].disabled&&boxes[1].checked,'new installed game selected');check(boxes[2].disabled,'unverified launch excluded');check(!q('#dialog').textContent.includes('Running entries'),'no process section');check(!q('#dialog [name=pid]'),'no PID imports');")
    snap('installed-game-discovery')
    js("click('#discovery-form [type=submit]')");pump()
    test('Installed import submits only selected installed entries',"const imports=bridge.calls.filter(c=>c.command==='import_discovered_games');check(imports.length===1&&JSON.stringify(imports[0].args.candidateIds)==='[\"steam:730\"]','installed identity submitted');check(!bridge.calls.some(c=>c.command==='import_running_games'||c.command==='running_game_candidates'),'no process commands');check(!q('#dialog').open,'import closes dialog');check(q('#toast').textContent==='1 game imported into the local library.','import feedback');click('[data-game=\"1\"]');")
    test('Catalog detail and launch locking',"check(q('[data-action=launch-game]').disabled,'active session launch gate');check(q('.library-game').textContent.includes('Test game'),'catalog');click('.game-more summary');click('[data-action=edit-launch]');check(q('[name=arguments]').value==='--one\\n--two','arguments displayed on separate lines');const a=q('.picker-field input').getBoundingClientRect(),b=q('.picker-field button').getBoundingClientRect();check(Math.abs(a.top-b.top)<1&&Math.abs(a.height-b.height)<1,'file picker alignment');")
    snap('launch-dialog')
    js("click('#dialog [type=submit]')");pump()
    test('Launch arguments round-trip',"check(bridge.launchArgs.length===2&&bridge.launchArgs[1]==='--two','separate args preserved');const update=bridge.calls.findLast(c=>c.command==='update_game_launch');check(update.args.expectedRevision==='launch-1','launch revision submitted')")
    test('Per-game visibility keeps inheritance and saves live feedback',"const input=q('[data-override=overlay]');check(input.getAttribute('aria-label')==='Show in-game overlay','game label');input.value='Off';input.dispatchEvent(new Event('change',{bubbles:true}));click('[data-action=save-game]');",350)
    test('Per-game saved reply refreshes the catalog',"check(q('[data-override=overlay]').value==='Off','saved off retained');check(q('#toast').textContent.includes('overlay was updated'),'live feedback');click('[data-action=reset-game]');click('[data-action=save-game]');",350)
    test('Reset to global restores visibility inheritance',"check(q('[data-override=overlay]').value==='inherit','inheritance restored');")
    test('Library has no retired replay override',"check(!q('[data-override=replay]'),'automatic replay has no off selector');const input=q('[data-override=overlay]');input.value='Off';input.dispatchEvent(new Event('change',{bubbles:true}));click('[data-game=\"2\"]');const other=q('[data-override=overlay]');other.value='On';other.dispatchEvent(new Event('change',{bubbles:true}));click('[data-action=save-game]');",350)
    test('Saving another game preserves the first pending draft',"click('[data-game=\"1\"]');check(q('[data-override=overlay]').value==='Off','first draft retained');check(!q('.game-save-bar').hidden,'still unsaved');click('.game-more summary');click('[data-action=edit-launch]');click('#dialog [type=submit]');",350)
    test('Saving launch details preserves pending profile settings',"check(q('[data-override=overlay]').value==='Off','launch refresh keeps draft');check(!q('.game-save-bar').hidden,'launch save does not commit profile');click('[data-action=discard-game]');check(q('[data-override=overlay]').value==='inherit','discard uses saved baseline');")
    test('Precision dropdown keyboard commits a game override',"window.trigger=q('[data-override=overlay]').nextElementSibling;trigger.focus();trigger.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));trigger.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));check(q('[data-override=overlay]').value==='Off','selected override');check(document.activeElement.classList.contains('precision-select-trigger'),'replacement focus');check(!q('[data-action=save-game]').disabled,'dirty save');click('[data-action=discard-game]');check(q('[data-override=overlay]').value==='inherit','discard restored inheritance');")
    test('Library search survives selection',"const search=q('#game-search');search.value='local';search.dispatchEvent(new Event('input',{bubbles:true}));document.querySelectorAll('[data-game]')[1].click();check(q('#game-search').value==='local','filter retained');")
    pump(200)
    test('Missing banner keeps neutral header without poster fallback',"check(!q('.game-cover-banner img'),'no portrait substituted for missing banner');check(q('.game-cover').getBoundingClientRect().height<260,'missing art keeps header compact');")
    snap('library')
    js("click('[data-game=\"1\"]');bridge.idle=true;window.dispatchEvent(new Event('focus'));");pump(2800)
    test('Installed game is launchable when no session owns the runtime',"check(!q('[data-action=launch-game]').disabled,'installed launch enabled');check(q('#game-installation-note').hidden,'no installed warning');const select=q('[data-override=overlay]');select.value='Off';select.dispatchEvent(new Event('change',{bubbles:true}));window.retainedBanner=q('.game-cover-banner img');window.retainedPoster=q('.library-game img');window.retainedDraft=q('[data-override=overlay]');bridge.installation={'1':'not_installed','2':'installed'};window.dispatchEvent(new Event('focus'));")
    pump(1600)
    test('Uninstall changes only launch availability and preserves draft and artwork',"const launch=q('[data-action=launch-game]'),note=q('#game-installation-note');check(launch.disabled,'launch stays disabled through runtime updates');check(note.textContent==='Not installed'&&!note.hidden,'not installed label');check(launch.getAttribute('aria-describedby')===note.id,'accessible explanation');check(note.getBoundingClientRect().top>launch.getBoundingClientRect().bottom,'note below launch');check(q('.game-cover-banner img')===retainedBanner&&q('.library-game img')===retainedPoster,'artwork nodes retained');check(q('[data-override=overlay]')===retainedDraft&&retainedDraft.value==='Off','draft retained');check(!q('[data-action=save-game]').disabled,'profile remains editable');check(getComputedStyle(q('.game-detail')).opacity==='1','entry is not dimmed');")
    snap('library-not-installed')
    js("bridge.installationFailure=true;window.dispatchEvent(new Event('focus'))");pump(300)
    test('Unreadable installation evidence is not mislabeled as uninstalled',"check(q('[data-action=launch-game]').disabled,'unknown launch blocked');check(q('#game-installation-note').textContent==='Installation status unavailable','accurate unknown label');bridge.installationFailure=false;bridge.installation={'1':'installed','2':'installed'};window.dispatchEvent(new Event('focus'));")
    pump(300)
    test('Reinstall restores launch without changing saved or draft settings',"check(!q('[data-action=launch-game]').disabled,'reinstalled launch enabled');check(q('#game-installation-note').hidden,'warning removed');check(q('[data-override=overlay]').value==='Off','draft still intact');click('[data-action=discard-game]');bridge.idle=false;")
    js('bridge.idle=true');pump(2800)
    route('global')
    test('HUD preview retains native FPS typography',"check(getComputedStyle(q('.hud-native-value')).fontSize==='24px','native 24px measurement font');")
    test('Clean global settings hide draft actions',"check(q('.settings-save-bar').hidden,'clean bar hidden')")
    snap('global-overlay')
    test('Layout and palette change structure and color without changing selected metrics',"const metrics=[...document.querySelectorAll('[data-metric]:checked')].map(input=>input.dataset.metric).join('|');const layout=q('[data-global=layout]');layout.value='Ribbon';layout.dispatchEvent(new Event('change',{bubbles:true}));const palette=q('[data-global=palette]');palette.value='Glacier';palette.dispatchEvent(new Event('change',{bubbles:true}));const hud=q('#hud'),brand=q('.hud-layout-brand'),brandText=q('.hud-layout-brand span'),hudBounds=hud.getBoundingClientRect(),textBounds=brandText.getBoundingClientRect();check(hud.dataset.layout==='Ribbon','ribbon preview');check(getComputedStyle(hud).width==='628px','ribbon fits six visible metrics');check(getComputedStyle(brand).flexDirection==='row','generic HUD rule does not top-align brand');check(Math.abs((textBounds.top+textBounds.bottom-hudBounds.top-hudBounds.bottom)/2)<1,'brand text vertically centered');check(hud.style.getPropertyValue('--hud-accent')===window.fixturePalettes.Glacier[0],'glacier accent');check([...document.querySelectorAll('[data-metric]:checked')].map(input=>input.dataset.metric).join('|')===metrics,'metrics unchanged');check(q('[data-layout=Ribbon]').getAttribute('aria-pressed')==='true','layout card selected');check(q('[data-palette=Glacier]').getAttribute('aria-pressed')==='true','palette swatch selected');")
    test('Every overlay palette controls both preview borders',"const expected=Object.fromEntries(Object.entries(window.fixturePalettes).map(([name,colors])=>[name,[colors[0],colors[4]].map(hex=>'rgb('+[1,3,5].map(offset=>parseInt(hex.slice(offset,offset+2),16)).join(', ')+')')]));const palette=q('[data-global=palette]'),hud=q('#hud');for(const [name,[accent,border]] of Object.entries(expected)){palette.value=name;palette.dispatchEvent(new Event('change',{bubbles:true}));const style=getComputedStyle(hud);check(style.borderLeftColor===accent,name+' accent border');check(style.borderTopColor===border,name+' panel border');}palette.value='Glacier';palette.dispatchEvent(new Event('change',{bubbles:true}));")
    snap('global-overlay-ribbon-glacier')
    test('Telemetry and Rose produce the dense preview',"const layout=q('[data-global=layout]');layout.value='Telemetry';layout.dispatchEvent(new Event('change',{bubbles:true}));const palette=q('[data-global=palette]');palette.value='Rose';palette.dispatchEvent(new Event('change',{bubbles:true}));check(q('#hud').dataset.layout==='Telemetry','telemetry preview');check(q('.hud-telemetry-row'),'dense metric row');check(q('#hud').style.getPropertyValue('--hud-accent')==='#ff82ad','rose accent');")
    snap('global-overlay-telemetry-rose')
    js("const layout=q('[data-global=layout]');layout.value='Grid';layout.dispatchEvent(new Event('change',{bubbles:true}));const palette=q('[data-global=palette]');palette.value='Redunar';palette.dispatchEvent(new Event('change',{bubbles:true}));check(q('.settings-save-bar').hidden,'restored style is clean')")
    test('Branding can be hidden without hiding metrics',"const branding=q('[data-global=branding]');branding.click();check(!q('#hud').textContent.includes('REDUNAR'),'branding removed from preview');check(q('.hud-native-frame'),'metrics remain visible');branding.click();check(q('#hud').textContent.includes('REDUNAR'),'branding restored');check(q('.settings-save-bar').hidden,'restored branding is clean');")
    test('Global toggles preserve booleans across input and change',"q('[data-global=overlay]').click();click('[data-action=save-global]')")
    pump()
    test('Boolean persisted through native payload',"const save=bridge.calls.findLast(c=>c.command==='save_global_settings');check(save.args.input.overlay===false,'boolean false persisted');check(save.args.expectedRevision==='1','optimistic revision preserved');check(q('#hud').hidden,'overlay off preview');check(q('[data-global=overlay]').getAttribute('aria-label')==='Show in-game overlay','clear visibility label');check(q('.settings-save-bar').hidden,'save hides bar');q('[data-global=overlay]').click();")
    test('Conflicts retain unsaved draft',"bridge.conflict=true;click('[data-action=save-global]')")
    pump()
    test('Conflict feedback and retry state',"check(q('#toast').textContent.includes('changed elsewhere'),'conflict surfaced');check(!q('[data-action=save-global]').disabled,'draft preserved');bridge.conflict=false;click('[data-action=discard-global]');check(!q('[data-global=overlay]').checked,'discard uses saved false');check(q('.settings-save-bar').hidden,'discard hides bar');")
    test('Preset and range update native preview',"const preset=q('[data-global=preset]');preset.value='Custom';preset.dispatchEvent(new Event('change',{bubbles:true}));check(!q('[data-metric=FPS]').disabled,'custom metrics enabled');const range=q('[data-global=scale]');range.value='150';range.dispatchEvent(new Event('input',{bubbles:true}));check(q('#hud').style.getPropertyValue('--hud-scale')==='1.5','native geometry scale');check(range.style.getPropertyValue('--range-fill')!=='','precision fill');")
    js("click('[data-global-tab=replay]')");pump()
    test('Draft survives global tab switch with the current recording modes',"check(!q('[data-action=save-global]').disabled,'draft retained across tabs');check([...q('[data-global=fps]').options].map(option=>option.value).join(',')==='60,240','60 and Variable modes; legacy rates are shown only when saved');check(!q('.global-grid'),'removed replay card leaves no empty grid column');const a=q('.replay-pref-actions').getBoundingClientRect(),above=q('.replay-pref-actions').previousElementSibling.getBoundingClientRect();check(a.top-above.bottom>=15,'folder action divider spacing')")
    test('Global draft actions remain at viewport bottom while scrolling',"const bar=q('.settings-save-bar'),root=q('.window-content');check(!bar.hidden,'dirty bar shown');root.scrollTo(0,root.scrollHeight);check(Math.abs(innerHeight-bar.getBoundingClientRect().bottom-16)<1,'fixed while scrolled');root.scrollTo(0,0);check(Math.abs(innerHeight-bar.getBoundingClientRect().bottom-16)<1,'fixed at page start');")
    snap('global-replay')
    js("click('[data-global-tab=shortcuts]')");pump()
    js("bridge.shortcutUnavailable=true");pump(2800);snap('shortcuts-unavailable')
    test('Shortcut failure has a retry action and retains unsaved bindings',"check(!q('[data-action=retry-shortcuts]').hidden,'retry available');check(q('#shortcut-feedback').textContent.includes('No readable keyboard'),'failure reason visible');const input=q('[data-shortcut]');window.previousShortcut=input.value;input.value='Ctrl+Alt+Z';input.dispatchEvent(new Event('input',{bubbles:true}));window.pendingShortcut=input.value;window.shortcutWrites=bridge.calls.filter(call=>call.command==='save_shortcuts').length;click('[data-action=retry-shortcuts]');",350)
    test('Retry activation avoids changing saved or pending shortcuts',"check(q('[data-action=retry-shortcuts]').hidden,'active retry hidden');check(q('[data-shortcut]').value===pendingShortcut,'draft preserved');check(bridge.calls.filter(call=>call.command==='save_shortcuts').length===shortcutWrites,'retry made no preference write');const input=q('[data-shortcut]');input.value=previousShortcut;input.dispatchEvent(new Event('input',{bubbles:true}));")
    test('All nine shortcut assignments remain accessible',"check(document.querySelectorAll('[data-shortcut]').length===9,'nine shortcuts');const shortcut=document.querySelectorAll('[data-shortcut]')[1];shortcut.focus();shortcut.dispatchEvent(new KeyboardEvent('keydown',{key:'F8',ctrlKey:true,bubbles:true}));click('[data-action=save-global]')")
    pump();test('Separate shortcut save does not discard profile draft',"check(bridge.calls.some(c=>c.command==='save_shortcuts'&&c.args.shortcuts['15']==='Ctrl+F8'),'shortcut payload');click('[data-global-tab=overlay]');check(!q('[data-action=save-global]').disabled,'profile still dirty');click('[data-action=discard-global]')")
    js("click('[data-global-tab=shortcuts]')");snap('global-shortcuts')
    test('Clear all removes every shortcut without changing replay settings',"click('[data-action=clear-shortcuts]');check([...document.querySelectorAll('[data-shortcut]')].every(i=>i.value===''),'all fields blank');click('[data-action=save-global]');",350)
    test('Empty shortcut set saves and disables keyboard activation',"const saved=bridge.calls.findLast(c=>c.command==='save_shortcuts');check(Object.values(saved.args.shortcuts).every(v=>!v),'empty assignments submitted');check(q('#toast').textContent.includes('All shortcuts cleared'),'clear feedback');check(!q('[data-action=save-global]')||q('[data-action=save-global]').disabled,'saved shortcuts clean');")
    snap('shortcuts-cleared')
    route('replay')
    test('Cleared save shortcuts leave no empty badge column',"check(q('.replay-shortcut-slot').hidden,'no reserved space without save bindings');check(q('#save-replay-shortcut').hidden,'no blank shortcut');check(!q('[data-action=save-replay]').disabled,'app save remains available');q('.replay-save-row').scrollIntoView({block:'center'});")
    pump();snap('replay-save-cleared')
    route('global');js("click('[data-global-tab=shortcuts]')");pump()
    test('Save shortcut works without a menu shortcut',"const input=document.querySelectorAll('[data-shortcut]')[1];input.focus();input.dispatchEvent(new KeyboardEvent('keydown',{key:'F8',bubbles:true}));click('[data-action=save-global]');",350)
    test('Menu can be reassigned after clearing all shortcuts',"const saved=bridge.calls.findLast(c=>c.command==='save_shortcuts');check(saved.args.shortcuts['15']==='F8'&&!saved.args.shortcuts.overlay,'save-only binding');const input=q('[data-shortcut]');input.focus();input.dispatchEvent(new KeyboardEvent('keydown',{key:'T',ctrlKey:true,shiftKey:true,bubbles:true}));click('[data-action=save-global]');",350)
    js("click('[data-global-tab=replay]')");pump()
    test('Automatic replay replaces the global off control',"check(!q('[data-global=replayEnabled]'),'no replay off control');check(q('#workspace').textContent.includes('Automatic'),'automatic status');q('[data-replay-preference=outside]').click();",350)
    test('Outside click preference does not resend a stale shortcut',"const write=bridge.calls.findLast(c=>c.command==='set_replay_overlay_behavior');check(Object.keys(write.args).length===1&&Object.hasOwn(write.args,'closeOnOutsideClick'),'only changed preference sent');check(values.shortcuts.overlay==='Ctrl+Shift+T','new menu shortcut retained');")
    js("bridge.idle=false;window.dispatchEvent(new Event('focus'))")
    wait_for("q('[data-replay-preference=microphone]').disabled")
    test('Microphone mode is locked during the current game session',"check(q('[data-replay-preference=microphone]').disabled,'active-session lock');check(q('#workspace').textContent.includes('end the current session'),'honest microphone timing');check(q('#workspace').textContent.includes('/fixture/Videos/Redunar Replays'),'visible save directory');bridge.idle=true;")
    wait_for("!q('[data-replay-preference=microphone]').disabled")
    test('Microphone toggle is busy until the native write completes',"q('[data-replay-preference=microphone]').click();check(q('[data-replay-preference=microphone]').disabled,'pending write disabled');check(!q('[data-replay-preference=microphone]').checked,'saved state retained while pending');",250)
    test('Microphone toggle recovers after success without changing tabs',"const input=q('[data-replay-preference=microphone]');check(input.checked&&!input.disabled,'success checked and interactive');input.focus();check(document.activeElement===input,'keyboard focus');check(q('#toast').textContent.includes('new sessions'),'next-session notice');input.click();",250)
    test('Microphone Off and native payload remain truthful',"check(!q('[data-replay-preference=microphone]').checked&&!q('[data-replay-preference=microphone]').disabled,'off and interactive');check(bridge.calls.findLast(c=>c.command==='set_replay_microphone').args.mode==='off','off payload');check(q('#toast').textContent.includes('disabled for new sessions'),'off notice');bridge.failMicrophone=true;q('[data-replay-preference=microphone]').click();",250)
    test('Microphone failed write restores saved state and retry availability',"check(!q('[data-replay-preference=microphone]').checked&&!q('[data-replay-preference=microphone]').disabled,'failed write recovered');check(q('#toast').textContent.includes('Fixture microphone write failed'),'honest failure');bridge.failMicrophone=false;")
    snap('microphone-normal')
    window.resize(640,950);pump()
    test('Microphone and save directory fit a compact workspace',"q('[data-replay-preference=microphone]').scrollIntoView({block:'center'});check(document.documentElement.scrollWidth<=innerWidth,'no compact overflow');")
    snap('microphone-compact')
    window.resize(1440,1000);pump()
    js("bridge.idle=true")
    route('history');snap('history')
    js("bridge.historyFixture=Array.from({length:20},(_,i)=>({id:i+1,game:'Scroll test '+i,started_unix:1789300000-i*86400,duration_seconds:1800,average_fps:60,timeline:[{elapsed_seconds:0,fps:60},{elapsed_seconds:1800,fps:60}]}));click('[data-action=reload-history]')");pump()
    test('History scrollbar sits between cards and the detail panel',"const rail=q('.history-catalog').getBoundingClientRect(),list=q('#session-list'),viewport=list.getBoundingClientRect(),card=q('.history-row').getBoundingClientRect(),detail=q('.history-detail').getBoundingClientRect();check(list.scrollHeight>list.clientHeight,'overflow fixture');check(viewport.right>rail.right+10,'scrollbar moved into gap');check(viewport.right<detail.left-2,'scrollbar stays clear of detail');check(card.right<rail.right,'cards stay inside rail');check(viewport.right-card.right>20,'scrollbar cannot overlay cards');")
    snap('history-scrollbar-gap')
    js("bridge.historyFixture=null;click('[data-action=reload-history]')");pump()
    test('History chart and inspector retain recorded data',"click('[data-history-view=temperature]');const slider=q('[data-history-cursor]');slider.value='1200';slider.dispatchEvent(new Event('input',{bubbles:true}));check(q('#history-moment-cpu-temp').textContent==='61.0°C','CPU sample');check(q('#history-moment-gpu-temp').textContent==='67.0°C','GPU sample');check(document.querySelectorAll('.timeline-trace').length===2,'both traces');")
    snap('history-temperatures')
    test('History labels align with grid lines and include units',"const svg=q('.history-timeline-chart svg');for(const label of document.querySelectorAll('.history-timeline-chart .y-axis span')){const grid=q('.gridline[data-axis-value=\"'+label.dataset.axisValue+'\"]');const y=Number(grid.getAttribute('d').split(' ')[1].split('H')[0]);const pt=new DOMPoint(0,y).matrixTransform(svg.getScreenCTM());const rect=label.getBoundingClientRect();check(Math.abs(rect.top+rect.height/2-pt.y)<1,'tick/grid mismatch');check(label.textContent.includes('°C'),'temperature unit');}check(q('.history-timeline-chart .y-axis').getBoundingClientRect().width>=70,'readable axis gutter');")
    test('History thumb, marker, output and selected observation agree',"const slider=q('[data-history-cursor]');for(const value of [0,900,1800]){slider.value=value;slider.dispatchEvent(new Event('input',{bubbles:true}));const cursor=Number(slider.value),svg=q('.history-timeline-chart svg'),marker=q('#history-cursor-line');const pt=new DOMPoint(Number(marker.getAttribute('x1')),0).matrixTransform(svg.getScreenCTM());const rect=slider.getBoundingClientRect(),thumb=rect.left+7+(rect.width-14)*cursor/1800;check(Math.abs(pt.x-thumb)<1,'thumb/marker mismatch '+value);check(q('#history-cursor-time').textContent===q('#history-slider-time').textContent+' into session','heading agrees');check(slider.getAttribute('aria-valuetext')===q('#history-cursor-time').textContent,'accessible time agrees');}slider.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));check(Math.abs(Number(slider.value)-94*1800/95)<.001,'previous actual observation');")
    test('History load axis is fixed at 0–100 percent',"click('[data-history-view=utilization]');check([...document.querySelectorAll('.y-axis span')].map(el=>el.textContent).join('|')==='0 %|25 %|50 %|75 %|100 %','load ticks');")
    snap('history-load')
    test('History graph retains all observations and actual FPS scale',"click('[data-history-view=fps]');const trace=q('.timeline-trace');check(trace.points.numberOfItems===96,'all journal observations retained');const ticks=[...document.querySelectorAll('.y-axis span')];check(ticks.at(-1).textContent==='150 FPS','raw FPS extrema determine scale');")
    snap('history-fps')
    js("q('#toast').hidden=true;q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-inspector');js("q('.window-content').scrollTo(0,0)")
    js("bridge.historyFixture=[{id:20,game:'Known temperature and load readings',started_unix:1789300000,duration_seconds:12,timeline:[0,3,9,12].map((elapsed_seconds,i)=>({elapsed_seconds,fps:[60,80,90,120][i],frame_time_ms:[10,20,5,12][i],cpu_temperature_celsius:[70,80,75,70][i],gpu_temperature_celsius:[75,72,80,75][i],cpu_utilization_percent:[25,80,75,100][i],gpu_utilization_percent:[60,40,80,0][i]}))}];click('[data-action=reload-history]')");pump()
    test('All History metrics plot the values shown by the inspector',r"""
      const cases=[['fps',['primary'],[[60,80,90,120]],['#history-moment-fps']],['frame-time',['primary'],[[10,20,5,12]],['#history-moment-frame']],['temperature',['cpu','gpu'],[[70,80,75,70],[75,72,80,75]],['#history-moment-cpu-temp','#history-moment-gpu-temp']],['utilization',['cpu','gpu'],[[25,80,75,100],[60,40,80,0]],['#history-moment-cpu-load','#history-moment-gpu-load']]];
      for(const [metric,kinds,values,readings] of cases){
        click('[data-history-view='+metric+']');
        const svg=q('.history-timeline-chart svg'),matrix=svg.getScreenCTM();
        const labels=[...document.querySelectorAll('.y-axis span')];
        const bottom=labels[0].getBoundingClientRect(),top=labels.at(-1).getBoundingClientRect();
        const bottomY=bottom.top+bottom.height/2,topY=top.top+top.height/2;
        const low=parseFloat(labels[0].textContent),high=parseFloat(labels.at(-1).textContent);
        for(const [index,time] of [0,3,9,12].entries()){
          const slider=q('[data-history-cursor]');slider.value=time;slider.dispatchEvent(new Event('input',{bubbles:true}));
          for(const [series,kind] of kinds.entries()){
            const trace=q('polyline.timeline-trace.'+kind),point=trace.points[index];
            const screen=new DOMPoint(point.x,point.y).matrixTransform(matrix);
            const plottedValue=low+(bottomY-screen.y)/(bottomY-topY)*(high-low);
            check(Math.abs(plottedValue-values[series][index])<.02,metric+' '+kind+' trace disagrees with left labels');
            check(parseFloat(q(readings[series]).textContent)===values[series][index],metric+' '+kind+' inspector disagrees');
            check(Math.abs(point.x-Number(q('#history-cursor-line').getAttribute('x1')))<.001,'cursor and observation disagree');
            if(values[series][index]===80&&(metric==='temperature'||metric==='utilization')){
              const tick=labels.find(el=>parseFloat(el.textContent)===75).getBoundingClientRect();
              check(screen.y<tick.top+tick.height/2,'80 must be ABOVE the 75 tick');
            }
          }
        }
      }
    """)
    test('CPU and GPU use consistent distinct red tints',"for(const metric of ['temperature','utilization']){click('[data-history-view='+metric+']');const cpu=getComputedStyle(q('.timeline-trace.cpu')).stroke,gpu=getComputedStyle(q('.timeline-trace.gpu')).stroke;check(cpu==='rgb(231, 71, 79)'&&gpu==='rgb(255, 146, 151)','red series colors');check(getComputedStyle(q('.history-chart-legend .cpu i')).backgroundColor===cpu,'CPU legend');check(getComputedStyle(q('.history-chart-legend .gpu i')).backgroundColor===gpu,'GPU legend');check(getComputedStyle(q('#history-moment-cpu-'+(metric==='temperature'?'temp':'load'))).color===getComputedStyle(q('#history-moment-fps')).color,'CPU reading keeps neutral text');check(getComputedStyle(q('#history-moment-gpu-'+(metric==='temperature'?'temp':'load'))).color===getComputedStyle(q('#history-moment-fps')).color,'GPU reading keeps neutral text');}")
    js("click('[data-history-view=temperature]');const slider=q('[data-history-cursor]');slider.value=3;slider.dispatchEvent(new Event('input',{bubbles:true}));q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-temperature-accuracy')
    js("click('[data-history-view=utilization]');q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-load-accuracy')
    js("q('.window-content').scrollTo(0,0);click('[data-history-view=fps]')")
    js("bridge.historyFixture=[{id:21,game:'Fractional load readings',started_unix:1789300000,duration_seconds:12,timeline:[0,3,9,12].map((elapsed_seconds,i)=>({elapsed_seconds,cpu_utilization_percent:[0,24.9999,25.0001,37.4256][i],gpu_utilization_percent:[10,20,30,40][i]}))}];click('[data-action=reload-history]')");pump()
    js("click('[data-history-view=utilization]')")
    test('Load uses session extrema and retains journal fractions at every cursor',r"""
      {
        const labels=[...document.querySelectorAll('.y-axis span')];
        check(labels.at(-1).textContent==='40 %','low-load scale follows session');
        const cpu=q('polyline.timeline-trace.cpu'),gpu=q('polyline.timeline-trace.gpu');
        const cpuValues=[0,24.9999,25.0001,37.4256],gpuValues=[10,20,30,40];
        for(const [i,time] of [0,3,9,12].entries()){
          const slider=q('[data-history-cursor]');slider.value=time;slider.dispatchEvent(new Event('input',{bubbles:true}));
          check(q('#history-moment-cpu-load').textContent===cpuValues[i]+'%','exact recorded CPU percentage');
          check(q('#history-moment-gpu-load').textContent===gpuValues[i]+'%','exact GPU percentage');
          for(const [line,value] of [[cpu,cpuValues[i]],[gpu,gpuValues[i]]]){
            const point=line.points[i];
            check(Math.abs((210-point.y)/190*40-value)<.00001,'load point and axis calculation agree');
            check(Math.abs(point.x-Number(q('#history-cursor-line').getAttribute('x1')))<.001,'load cursor visits exact observation');
          }
        }
      }
    """)
    js("q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-fractional-load');js("click('[data-history-view=fps]')")
    js("bridge.historyFixture=[{id:10,game:'Sparse recorded session',started_unix:1789300000,duration_seconds:20,timeline:[{elapsed_seconds:2,fps:60,frame_time_ms:16.67},{elapsed_seconds:3,fps:360,frame_time_ms:2.78},{elapsed_seconds:9,fps:null},{elapsed_seconds:12,fps:45,frame_time_ms:22.22}]}];click('[data-action=reload-history]')");pump()
    test('Sparse History preserves peaks, gaps and unavailable time',"check(q('.y-axis span:last-child').textContent==='400 FPS','spike scale');check(q('polyline.timeline-trace').points.numberOfItems===2,'gap splits trace');check(q('circle.timeline-observation')!==null,'isolated sample visible');check(Number(q('polyline.timeline-trace').points[0].x)===90,'no fabricated session-start point');const slider=q('[data-history-cursor]');slider.value=3;slider.dispatchEvent(new Event('input',{bubbles:true}));check(q('#history-moment-fps').textContent==='360.0','recorded peak');slider.value=0;slider.dispatchEvent(new Event('input',{bubbles:true}));check(q('#history-moment-fps').textContent==='—','no extrapolated FPS');check(q('#history-cursor-time').textContent==='00:00 into session','empty position time');")
    js("q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-gaps');js("q('.window-content').scrollTo(0,0)")
    js("bridge.historyFixture=[{id:11,game:'Legacy retained frames',started_unix:1789300000,duration_seconds:10,frame_intervals_ns:[500000,20000000]}];click('[data-action=reload-history]')");pump()
    test('Legacy History keeps fractional timestamps and uncapped FPS',"const slider=q('[data-history-cursor]');check(q('#history-moment-fps').textContent==='50.0','last retained frame');slider.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}));check(q('#history-moment-fps').textContent==='2000.0','uncapped reciprocal');check(q('#history-slider-time').textContent==='00:09.98','fractional retained time');slider.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));check(q('#history-moment-fps').textContent==='50.0','exact end after range round-trip');")
    js("bridge.historyFixture=[{id:12,game:'No observations',started_unix:1789300000,duration_seconds:0}];click('[data-action=reload-history]')");pump()
    test('Empty History disables the timeline without inventing numbers',"check(q('[data-history-cursor]').disabled,'disabled scrubber');check(q('#history-moment-fps').textContent==='—','unknown FPS');check(q('#history-slider-time').textContent==='00:00','zero duration');check(!q('#history-cursor-line'),'no fabricated graph');bridge.historyFixture=null;click('[data-action=reload-history]');")
    pump()
    route('settings');snap('settings')
    test('Debug log distinguishes saved opt-in from runtime health',"check(q('#diagnostic-log-status').textContent==='Logging is off.','actual logger off');q('[data-preference=diagnostic-log]').click()")
    pump();test('Debug log enable requires restart',"check(q('#diagnostic-log-status').textContent.includes('Restart Redunar'),'preference alone is not active');bridge.logStatus='active';")
    pump(2800);test('Active logging is observed from native status',"check(q('#diagnostic-log-status').textContent.includes('active on this device'),'live health');bridge.failDaemon=true;")
    pump(2800);test('Rejected native status clears stale active logging',"check(!q('#diagnostic-log-status').textContent.includes('active on this device'),'stale active cleared');check(q('#diagnostic-log-status').textContent.includes('unavailable'),'unknown health visible');bridge.failDaemon=false;bridge.logStatus='unavailable';")
    pump(2800);test('Write failure appears while saved switch stays enabled',"check(q('#diagnostic-log-status').textContent.includes('Logging is unavailable'),'failure visible');check(q('[data-preference=diagnostic-log]').checked,'saved opt-in retained');")
    snap('settings-log-unavailable')
    window.resize(980,740);pump();snap('settings-log-unavailable-compact')
    test('Logger failure remains readable at compact width',"check(document.documentElement.scrollWidth<=innerWidth+1,'no horizontal overflow');check(q('#diagnostic-log-status').textContent.includes('Logging is unavailable'),'failure retained');")
    window.resize(1440,1000);pump()

    js("bridge.logStatus='off';q('[data-preference=diagnostic-log]').click()");pump(1400)

    test('Settings exposes only current preferences',"check(!q('[data-module]'),'no module switches');check(!q('[data-preference=motion]'),'no motion toggle');check(!q('[data-action=reload-preferences]'),'no reload button');check(!q('#workspace').textContent.includes('Feature modules'),'no module section');check(!q('.app-about'),'no redundant about card');check(!q('#workspace').textContent.includes('Measurement & storage'),'no fixed implementation counters');check(q('[data-preference=automatic-updates]').checked,'automatic updates default on');check(q('[data-action=install-update]').textContent==='Update now','startup update remains actionable');q('[data-preference=tray]').click()")
    pump();test('Tray persists through the native command',"check(bridge.calls.some(c=>c.command==='set_close_to_tray'&&c.args.enabled===true),'tray save');check(q('[data-preference=tray]').checked,'saved tray state');")
    js("q('[data-preference=tray]').click()");pump()
    test('Disabling tray sends the native removal immediately',"check(bridge.calls.findLast(c=>c.command==='set_close_to_tray').args.enabled===false,'native removal requested');check(!q('[data-preference=tray]').checked,'disabled state shown');")
    js("click('[data-action=install-update]')");pump()
    test('Verified startup update keeps retry available',"check(bridge.calls.some(c=>c.command==='install_update'),'native update request');check(q('[data-action=install-update]').textContent==='Retry update','unconfirmed install can retry');check(q('[data-action=check-for-updates]').textContent==='Check again','manual recheck remains available');check(q('[data-update-state]').textContent.includes('Installation unconfirmed'),'installer exit is not success');check(q('.updates-heading .pill').textContent==='Version 0.1.2','current version retained');check(q('#toast').textContent.includes('could not confirm'),'update feedback');")
    snap('settings-handoff')
    pump(700);js("bridge.installedUpdate=true;click('[data-action=check-for-updates]')");pump()
    test('Installed package requires restart without offering a second install',"check(q('[data-update-state]').textContent==='Package installed · restart Redunar','installed package recognized');check(!q('[data-action=install-update]'),'cannot reinstall completed package');check(q('[data-action=check-for-updates]'),'recheck available');check(bridge.calls.findLast(c=>c.command==='check_for_updates').args.refreshPending===true,'manual check requests newest signed release')")
    snap('settings-restart-needed')
    js('bridge.installedUpdate=false')
    route('overview');route('settings')
    js("q('[data-preference=automatic-updates]').click()");pump(500)
    test('Automatic update preference persists independently',"check(bridge.calls.findLast(c=>c.command==='set_automatic_updates').args.enabled===false,'automatic checks disabled');check(!q('[data-preference=automatic-updates]').checked,'saved update preference shown');click('[data-action=check-for-updates]');")
    pump();test('Manual update check reports authenticated-source readiness honestly',"check(bridge.calls.some(c=>c.command==='check_for_updates'),'manual native check');check(q('[data-update-state]').textContent==='Source pending','honest readiness state');check(q('#toast').textContent.includes('not configured'),'honest check feedback');")
    snap('settings-updates')
    route('replay');snap('replay-loading')
    test('Replay cards lead with the recording game',"const first=q('[data-clip]');check(first.querySelector('.clip-game').textContent===games[0].name,'game is primary label');check(first.querySelector('.clip-file').textContent.includes('redunar-replay'),'filename remains available');check(q('.native-video-caption strong').textContent===games[0].name,'selected player names game');")
    test('Obsolete replay preview controls are absent',"check(!q('[data-action=preview-replay-menu]'),'no preview action');check(!q('#replay-preview'),'no preview dialog');")
    route('overview');js('bridge.unavailable=true');wait_for("q('#ram-used').textContent==='—'");test('Unavailable RAM never displays stale measurements',"check(q('#ram-used').textContent==='—','RAM cleared');check(q('#ram-meter').style.width==='0%','unavailable meter cleared');bridge.unavailable=false;")
    window.resize(1040,900);pump();route('history');js("q('#toast').hidden=true;q('.history-timeline-chart').scrollIntoView({block:'start'})");snap('history-compact')
    test('History remains readable at compact width',"check(document.documentElement.scrollWidth<=innerWidth+1,'history overflow');check(q('.history-timeline-chart svg').getBoundingClientRect().width>200,'plot remains visible');")
    route('global');snap('global-compact')
    test('Compact window does not overflow horizontally',"check(document.documentElement.scrollWidth<=innerWidth+1,'horizontal overflow');check(bridge.errors.length===0,bridge.errors.join('; '));")
    js("click('[data-global-tab=overlay]');q('[data-global=overlay]').click();q('#toast').hidden=true")
    test('Compact global draft actions remain at window bottom',"const rect=q('.settings-save-bar').getBoundingClientRect();check(Math.abs(innerHeight-rect.bottom-16)<1,'compact bottom');check(rect.right<=innerWidth,'compact right edge');")
    snap('global-dirty-compact')
    window.resize(640,720);pump()
    test('Narrow draft actions fit the viewport and hide after discard',"const bar=q('.settings-save-bar'),rect=bar.getBoundingClientRect();check(Math.abs(innerHeight-rect.bottom-12)<1,'narrow bottom');for(const button of bar.querySelectorAll('button')){const b=button.getBoundingClientRect();check(b.left>=0&&b.right<=innerWidth&&b.bottom<=innerHeight,'visible action');}check(document.documentElement.scrollWidth<=innerWidth+1,'narrow overflow');")
    snap('global-dirty-narrow')
    js("click('[data-action=discard-global]')")
    test('Discard removes the narrow action bar',"check(q('.settings-save-bar').hidden,'bar hidden');check(document.body.dataset.unsaved==='false','content clearance removed');")
    # A running game makes visibility an immediate, isolated save.
    window.resize(1440,1000)
    view.load_uri(f'http://127.0.0.1:{server.server_port}/#global');pump(1800)
    js("q('[data-global=scale]').value='105';q('[data-global=scale]').dispatchEvent(new Event('input',{bubbles:true}));click('[data-global-tab=replay]');q('[data-global=fps]').value='240';q('[data-global=fps]').dispatchEvent(new Event('change',{bubbles:true}));click('[data-global-tab=overlay]');q('[data-global=overlay]').click();")
    pump(400)
    test('Live visibility saves immediately without committing pending recording or appearance edits',"const save=bridge.calls.findLast(c=>c.command==='save_global_settings');check(save.args.input.overlay===false,'hide persisted without Save button');check(save.args.input.fps===60&&save.args.input.scale===100,'other draft values excluded');check(q('[data-global=scale]').value==='105','appearance draft retained');check(!q('.settings-save-bar').hidden,'other drafts still pending');check(!q('[data-global=overlay]').checked,'hidden');check(!q('[data-global=overlay]').disabled,'can show again');")
    snap('global-live-overlay-hidden')
    js("q('[data-global=overlay]').click()");pump(400)
    test('Live overlay can be shown again without recording reconfiguration',"check(bridge.calls.findLast(c=>c.command==='save_global_settings').args.input.overlay===true,'show persisted');check(q('[data-global=overlay]').checked,'shown');check(q('[data-global=scale]').value==='105','draft still intact');bridge.conflict=true;q('[data-global=overlay]').click();")
    pump(400)
    test('Failed live visibility save restores the saved toggle and preserves drafts',"check(q('[data-global=overlay]').checked,'last saved shown state restored');check(!q('[data-global=overlay]').disabled,'retry usable');check(q('#toast').textContent.includes('changed elsewhere'),'error visible');check(q('[data-global=scale]').value==='105','draft retained');bridge.conflict=false;click('[data-action=discard-global]');check(q('[data-global=overlay]').checked,'discard does not undo saved visibility');check(q('.settings-save-bar').hidden,'draft bar cleared');")
    (ARTIFACTS/'workspace-checks.json').write_text(json.dumps(checks,indent=2)+'\n')
    print(f'PASS: {len(checks)} workspace checks. Screenshots: {ARTIFACTS}')
finally:
    window.destroy();server.shutdown()
