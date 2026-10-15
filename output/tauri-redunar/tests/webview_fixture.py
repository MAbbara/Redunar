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
ARTIFACTS=NATIVE.parents[1]/'.redunar-build/reports/workspace-ui'
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
function replayPreferencesFixture(){return {microphone_mode:bridge.microphone||'off',microphone_source:bridge.source||null,initial_save_duration_seconds:30,resolved_directory:(bridge.folder??'/fixture/Videos')+'/Redunar Replays',custom_save_parent:bridge.folder??null,close_overlay_on_outside_click:bridge.outsideClick!==false};}
const games=[{id:'1',name:'Test game with a long local catalog title',executable:'/usr/bin/steam',arguments:['--one','--two'],working_directory:null,steam_app_id:42,revision:'1',launch_revision:'launch-1',overrides:{}},{id:'2',name:'Second local game',executable:'/fixture/second',arguments:[],revision:'1',launch_revision:'launch-2',overrides:{overlay:false}}];
const catalogCase=new URLSearchParams(location.search).get('catalog');
if(catalogCase){bridge.catalog=catalogCase==='populated'?games:[];bridge.idle=true;bridge.startupUpdateAvailable=false;bridge.catalogPending=catalogCase==='pending';bridge.catalogFailure=catalogCase==='failed';}
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
 if(command==='import_discovered_games'){await new Promise(resolve=>setTimeout(resolve,180));if(bridge.failImport)throw Error('Another Redunar app owns the backend session. Close it, then reopen this app before editing saved data.');return structuredClone(games);}
 if(command==='catalog_games'){if(bridge.catalogPending)await new Promise(resolve=>bridge.releaseCatalog=resolve);if(bridge.catalogFailure)throw Error('Fixture catalog unavailable');return structuredClone(bridge.catalog??games);}
 if(command==='game_installation_statuses'){if(bridge.installationFailure)throw Error('Fixture library unavailable');return bridge.installation||{'1':'installed','2':'installed'};}
 if(command==='game_poster')return posterFixture;
 if(command==='game_banner')return args.gameId==='1'?bannerFixture:null;
 if(command==='replay_clips')return bridge.clips??=Array.from({length:3},(_,i)=>({file_name:`redunar-replay-1789254367-629872093-${i+1}-a-long-capture-filename.mkv`,title:`Capture ${i+1}`,game_name:i===0?games[0].name:null,bytes:68000000,modified_unix_ns:'1789300000000000000',duration_seconds:30}));
 if(command==='clip_metadata')return {duration_seconds:30,width:640,height:360,fps:60};
 if(command==='clip_thumbnail')return captureFixture;
 if(command==='replay_storage_status')return {used_bytes:204000000,available_bytes:bridge.lowDisk?950000000:137400000000,total_bytes:512000000000};
 if(command==='microphone_inputs'){if(bridge.failInputs)throw Error('Fixture audio server unavailable');return [{id:'usb-mic',label:'USB microphone'},{id:'internal-mic',label:'Internal microphone'}];}
 if(command==='pick_replay_folder')return bridge.pickedFolder??null;
 if(command==='set_replay_save_parent'){bridge.folder=args.parent;return replayPreferencesFixture();}
 if(command==='set_clip_details'){await new Promise(r=>setTimeout(r,80));if(bridge.failFavorite)throw Error('Fixture favorite persistence failed');const c=bridge.clips.find(c=>c.file_name===args.fileName);c.favorite=args.favorite;return {file_name:c.file_name,favorite:c.favorite};}
 if(command==='rename_replay_clip'){await new Promise(r=>setTimeout(r,80));if(bridge.failRename)throw Error('A clip with that filename already exists');const c=bridge.clips.find(c=>c.file_name===args.fileName);c.file_name=args.newFileName;return {file_name:c.file_name,favorite:Boolean(c.favorite)};}
 if(command==='app_preferences')return {close_to_tray:false,automatic_updates:bridge.automaticUpdates,diagnostic_log:bridge.logRequested===true,diagnostic_log_status:bridge.logStatus||'off'};
 if(command==='set_close_to_tray')return {close_to_tray:args.enabled,automatic_updates:bridge.automaticUpdates};
 if(command==='set_diagnostic_log'){bridge.logRequested=args.enabled;return {diagnostic_log:args.enabled,diagnostic_log_status:bridge.logStatus||'off'};}
 if(command==='set_automatic_updates'){bridge.automaticUpdates=args.enabled;return {close_to_tray:false,automatic_updates:bridge.automaticUpdates};}
 if(command==='check_for_updates'){
  if(bridge.installedUpdate)return {current_version:'0.1.2',state:'restart-needed',message:'Redunar 0.1.3 is installed. Fully quit Redunar, including its tray process, then reopen it to load the update.',latest_version:'0.1.3',package_kind:'arch',asset_name:'redunar-app-linux-x86_64.pkg.tar.zst'};
  if(bridge.startupUpdateAvailable){bridge.startupUpdateAvailable=false;return {current_version:'0.1.2',state:'available',message:'Redunar 0.1.3 has a verified update package.',latest_version:'0.1.3',package_kind:'arch',asset_name:'redunar-app-linux-x86_64.pkg.tar.zst'};}
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
 if(command==='set_replay_microphone'){await new Promise(resolve=>setTimeout(resolve,80));if(bridge.failMicrophone)throw Error('Fixture microphone write failed');bridge.microphone=args.mode;bridge.source=args.source||null;return replayPreferencesFixture();}
 if(command==='replay_preferences')return replayPreferencesFixture();
 if(command==='replay_display_capability')return {compatible_120_modes:0,status:'No compatible display'};
 if(command==='activate_shortcuts')bridge.shortcutUnavailable=false;
 if(command==='shortcut_status'&&bridge.keyboardWaiting)return {state:'Unavailable',message:'No readable keyboard input device. Waiting for keyboard access to return.'};
 if(command==='shortcut_status'&&bridge.shortcutUnavailable)return {state:'Unavailable',message:'No readable keyboard input device'};
 if(command==='shortcut_status'||command==='activate_shortcuts')return {state:Object.values(values.shortcuts).some(Boolean)?'Active':'Inactive'};
 if(command==='session_history')return bridge.historyFixture??[0,1,2].map((n)=>({id:n+1,game:n===2?'Other local game':games[0].name,started_unix:1789300000-n*86400,duration_seconds:1800,average_fps:141-n,one_percent_low_fps:118,point_one_percent_low_fps:97,frame_intervals_ns:[7100000,7300000],timeline:Array.from({length:96},(_,i)=>({elapsed_seconds:i*1800/95,fps:135+Math.sin(i)*6,frame_time_ms:7.1+Math.sin(i)*.3,cpu_temperature_celsius:61,gpu_temperature_celsius:67,cpu_utilization_percent:32,gpu_utilization_percent:81}))}));
 return {};
}};
window.check=(condition,message)=>{if(!condition)throw new Error(message)};
window.q=s=>document.querySelector(s);window.qa=s=>Array.from(document.querySelectorAll(s));
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
