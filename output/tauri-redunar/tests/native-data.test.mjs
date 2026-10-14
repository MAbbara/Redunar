import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {storageSummary} from '../ui/workspace-improvements.mjs';
import {mapGames,mapClips,mapSessions,measurement,overridePayload,mapDefaults,profilePayload,profileDraftChanged,shortcutDraftChanged,overrideDraftChanged,visibleSession} from '../ui/native-data.mjs';

test('ended unlocked sessions are hidden from the live Overview while pending sessions remain visible',()=>{
 assert.equal(visibleSession({phase:'Ended',can_end:false,launch_locked:false,game:'Finished game'}),null);
 const pending={phase:'Ended',can_end:false,launch_locked:true,game:'Finishing game'};
 assert.deepEqual(visibleSession(pending),pending);
 const active={phase:'Running',can_end:true,launch_locked:false,game:'Active game'};
 assert.deepEqual(visibleSession(active),active);
});

test('release assets publish the VERSION metadata consumed by the updater',()=>{
 const workflow=readFileSync(new URL('../../../.github/workflows/release.yml',import.meta.url),'utf8');
 const candidate=readFileSync(new URL('../../../tools/prepare-release.sh',import.meta.url),'utf8');
 const signer=readFileSync(new URL('../../../tools/sign-release-assets.sh',import.meta.url),'utf8');
 assert.match(workflow,/candidate\/VERSION/);
 assert.match(candidate,/tools\/write-release-version\.sh "\$assets"/);
 assert.match(signer,/sha256sum VERSION/);
});

test('an empty backend remains empty; unknown measurements never become zero',()=>{
 assert.deepEqual(mapGames([]),[]);assert.deepEqual(mapClips([]),[]);assert.deepEqual(mapSessions([]),[]);
 for(const invalid of [undefined,null,NaN,Infinity,'80'])assert.equal(measurement(invalid),'—');
 assert.equal(measurement(0),'0');
});
test('history preserves observations, timeline telemetry, and caps the legacy frame sequence',()=>{
 const [record]=mapSessions([{game:'Local game',started_unix:0,duration_seconds:62,average_fps:80,one_percent_low_fps:null,point_one_percent_low_fps:null,frame_intervals_ns:[0,NaN,...Array(250).fill(12500000)],timeline:[{elapsed_seconds:2,fps:79.5,frame_time_ms:12.6,cpu_temperature_celsius:61,gpu_temperature_celsius:69,cpu_utilization_percent:32,gpu_utilization_percent:94}]}]);
 assert.equal(record.duration,'1m 2s');assert.equal(record.fps,'80');assert.equal(record.averageFps,80);assert.equal(record.low,'—');
 assert.equal(record.frames.length,240);assert.equal(record.frames[0],12.5);
 assert.deepEqual(record.timeline[0],{elapsed:2,fps:79.5,frameTime:12.6,cpuTemperature:61,gpuTemperature:69,cpuUtilization:32,gpuUtilization:94});
 assert.equal(record.gpu,undefined);assert.equal(record.restored,undefined);
});
test('history presents the newest recorded sessions first',()=>{
 const records=mapSessions([
  {id:4,game:'Older',started_unix:200,duration_seconds:1,average_fps:60,one_percent_low_fps:null,point_one_percent_low_fps:null},
  {id:5,game:'Newer',started_unix:300,duration_seconds:1,average_fps:90,one_percent_low_fps:null,point_one_percent_low_fps:null},
 ]);
 assert.deepEqual(records.map(record=>record.game),['Newer','Older']);
});
test('History renders a scrubber with FPS, temperatures, and hardware load',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 assert.match(source,/from '.\/history-timeline.mjs'/);
 assert.match(source,/data-history-cursor/);
 assert.match(source,/CPU temperature/);
 assert.match(source,/GPU temperature/);
 assert.match(source,/Long sessions retain the full time span/);
 assert.match(css,/\.history-moment-grid/);
 assert.match(css,/\.history-cursor-line/);
});
test('History timeline uses a stable primary trace and bounds the area to recorded samples',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const layout=readFileSync(new URL('../ui/app.css',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 assert.match(source,/history-area-gradient/);
 assert.match(source,/function aggregateTimelinePoints\(/);
 assert.match(source,/Math\.sqrt\(points\.length\)\*2/);
 assert.match(source,/const middle=Math\.floor\(values\.length\/2\)/);
 assert.doesNotMatch(source,/history-trace-gradient/);
 assert.doesNotMatch(css,/history-glow/);
 assert.match(css,/\.timeline-area\{fill:url\(#history-area-gradient\)/);
 assert.match(layout,/\.history-workspace\{align-items:stretch;min-height:0\}/);
 assert.match(css,/\.history-catalog\{align-self:stretch;display:flex;height:100%;min-height:0;overflow:hidden;flex-direction:column;justify-content:flex-start\}/);
 assert.match(css,/\.history-catalog #session-list\{flex:0 0 auto;align-self:stretch;width:100%;height:min\(930px,calc\(100vh - 220px\)\);max-height:min\(930px,calc\(100vh - 220px\)\);min-height:0;overflow-y:auto;padding-right:14px;scrollbar-gutter:stable/);
 assert.match(css,/\.history-row\{height:94px;min-height:94px;box-sizing:border-box;padding:13px 16px;margin-bottom:7px;overflow:hidden\}/);
 assert.match(css,/@media\(max-width:980px\)\{\.history-catalog #session-list\{height:auto;max-height:none;overflow:visible\}/);
});
test('Overview timeline shares the stable trace and baseline-anchored area treatment',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 assert.match(source,/overview-timeline-chart/);
 assert.match(source,/LIVE_TIMELINE_LIMIT = 240/);
 assert.match(source,/liveTimeline\.values\.push\(newest\/1e6\)/);
 assert.match(source,/overviewMetric==='fps'\?liveTimeline\.values\.map\(value=>1000\/value\):liveTimeline\.values/);
 assert.match(source,/aggregateTimelinePoints\(values\.map\(\(value,index\)=>\(\{x:index,value\}\)\)/);
 assert.match(source,/live\?'Older':'Sample 1'/);
 assert.match(source,/live\?'Latest sample':`Sample \$\{values\.length\}`/);
 assert.match(source,/overview-area-gradient/);
 assert.match(source,/0,210 \$\{points\} 900,210/);
 assert.match(css,/\.overview-area\{fill:url\(#overview-area-gradient\)/);
 assert.match(css,/\.overview-timeline-chart \.trace\{stroke:#ef5a6c;stroke-width:2\.4;/);
});
test('Desktop launcher metadata uses the production tooltip copy',()=>{
 const desktop=readFileSync(new URL('../../../packaging/com.redunar.Redunar.desktop',import.meta.url),'utf8');
 const metainfo=readFileSync(new URL('../../../packaging/com.redunar.Redunar.metainfo.xml',import.meta.url),'utf8');
 const tauri=JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json',import.meta.url),'utf8'));
 const installer=readFileSync(new URL('../install-desktop.py',import.meta.url),'utf8');
 assert.match(desktop,/^Name=Redunar$/m);
 assert.match(desktop,/^Comment=App for instant replay and in-game metrics\.$/m);
 assert.match(desktop,/^Icon=com\.redunar\.Redunar$/m);
 assert.match(desktop,/^StartupWMClass=com\.redunar\.Redunar$/m);
 assert.equal(tauri.identifier,'com.redunar.Redunar');
 assert.match(metainfo,/<id>com\.redunar\.Redunar<\/id>/);
 assert.match(metainfo,/<launchable type="desktop-id">com\.redunar\.Redunar\.desktop<\/launchable>/);
 assert.match(installer,/APP_ID = "com\.redunar\.Redunar"/);
 assert.match(installer,/Comment=App for instant replay and in-game metrics\./);
 assert.doesNotMatch(desktop,/^Name=.*Tauri|^Comment=.*Tauri|^Comment=.*native gaming workspace/m);
});
test('Embedded sidebar product version is generated from the package version',()=>{
 const index=readFileSync(new URL('../ui/index.html',import.meta.url),'utf8');
 const config=readFileSync(new URL('../vite.config.js',import.meta.url),'utf8');
 const build=readFileSync(new URL('../build-native.py',import.meta.url),'utf8');
 assert.match(index,/Version __REDUNAR_VERSION__/);
 assert.match(config,/packaging\/redunar-app\.spec/);
 assert.match(config,/replaceAll\('__REDUNAR_VERSION__', version\)/);
 assert.doesNotMatch(build,/index\.write_text/);
});
test('Overview uses ended language after a completed game session',()=>{
 const source=readFileSync(new URL('../src-tauri/src/sessions.rs',import.meta.url),'utf8');
 assert.match(source,/Game session ended\./);
 assert.doesNotMatch(source,/Game session completed\./);
});
test('inheritance is explicit and omits unsupported profile fields',()=>{
 assert.deepEqual(overridePayload({overlay:false,removedFeature:true,captureMetrics:true}),{overlay:false,captureMetrics:true});
 assert.deepEqual(overridePayload({}),{overlay:null,captureMetrics:null});
});
test('catalog identifiers retain precision and edits do not mutate backend response',()=>{
 const source=[{id:'18446744073709551615',name:'Game',executable:'/game',overrides:{overlay:false}}];
 const [game]=mapGames(source);game.overrides.overlay=true;
 assert.equal(game.id,source[0].id);assert.equal(source[0].overrides.overlay,false);
});
test('defaults round-trip real backend values, including frame metrics, zero opacity and temperatures',()=>{
 const values={overlay:false,preset:'Custom',layout:'Telemetry',palette:'Glacier',position:'Bottom right',scale:200,opacity:0,metrics:['GPU temperature'],captureMetrics:false,replayEnabled:false,fps:30,quality:'Efficient',format:'MP4',storageLimit:'Unlimited',shortcuts:{overlay:'Shift+Tab','30':'F8'}};
 const result=profilePayload(mapDefaults({values}),values.shortcuts);
 for(const key of Object.keys(values))assert.deepEqual(result[key],values[key]);
});
test('final settings surface omits internal runtime and diagnostics panels',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const start=source.indexOf('function settings()');
 const end=source.indexOf('function diagnosticsCard()',start);
 assert.ok(start>=0&&end>start,'settings renderer should remain explicit');
 const settingsSource=source.slice(start,end);
 assert.doesNotMatch(settingsSource,/Runtime connections/);
 assert.doesNotMatch(settingsSource,/diagnosticsCard\(\)/);
});
test('obsolete desktop replay preview is absent',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const html=readFileSync(new URL('../ui/index.html',import.meta.url),'utf8');
 assert.doesNotMatch(source,/Preview replay menu|preview-replay-menu|replay-preview|function replayMenu\(\)/);
 assert.doesNotMatch(html,/replay-preview/);
 assert.doesNotMatch(source,/capture-explainer|Local replay buffer|Replay controls/);
});
test('desktop replay menu host remains retired',()=>{
 assert.throws(()=>readFileSync(new URL('../src-tauri/src/replay_menu_window.rs',import.meta.url),'utf8'));
 assert.throws(()=>readFileSync(new URL('../ui/replay-menu.html',import.meta.url),'utf8'));
 const hotkeys=readFileSync(new URL('../src-tauri/src/hotkeys.rs',import.meta.url),'utf8');
 assert.doesNotMatch(hotkeys,/WebviewWindowBuilder|replay_menu_window/);
});
test('global overlay preview includes the native metric hierarchy',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const start=source.indexOf('function overlayPreview()');
 const end=source.indexOf('function replay()',start);
 assert.ok(start>=0&&end>start,'overlay preview renderer should remain explicit');
 const previewSource=source.slice(start,end);
 assert.match(previewSource,/data-preset/);
 assert.match(previewSource,/hud-native-header/);
 assert.match(previewSource,/hud-native-frame/);
 assert.match(previewSource,/hud-native-lows/);
 assert.match(previewSource,/hud-native-hardware/);
 assert.match(previewSource,/--hud-native-height/);
 assert.match(previewSource,/In-game overlay layout/);
 assert.match(previewSource,/preset==='FPS only'/);
 assert.match(previewSource,/144/);
 assert.match(previewSource,/6\.9/);
 assert.doesNotMatch(previewSource,/\^C/);
});
test('global preview applies select changes and keeps FPS only on one row',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/app.css',import.meta.url),'utf8');
 assert.match(source,/if\(t\.dataset\.global\)\{\n  const key=t\.dataset\.global;/);
 assert.match(source,/if\(next==='Custom' && draft\.preset!=='Custom'\)draft\.metrics=effectiveMetrics\(/);
 assert.match(source,/input\.checked=shown\.has\(input\.dataset\.metric\);input\.disabled=!custom/);
 assert.match(source,/draft\.metrics=t\.checked\?\[\.\.\.new Set\(\[\.\.\.draft\.metrics,t\.dataset\.metric\]\)\]/);
 assert.match(source,/const emptyCustom=globalTab==='overlay'&&draft\.preset==='Custom'&&!draft\.metrics\.length/);
 assert.match(source,/Select at least one metric to save a Custom layout/);
 assert.match(source,/Preset controlled · choose Custom to edit individual metrics/);
 assert.match(source,/25\+\(frame\?38:0\)\+\(lows\?26:0\)\+\(hardware\?26:0\)/);
 assert.match(css,/.hud-native-frame\{position:relative;height:38px/);
 assert.match(css,/.hud-native-value\{position:absolute;top:6px/);
 assert.match(css,/.hud-native-fps-label\{left:70px\}/);
 assert.match(css,/.hud-native-frame-time-label\{left:242px\}/);
 assert.match(css,/.hud-native-left,.hud-native-right\{position:absolute;top:6px/);
 assert.match(css,/\.hud\[data-preset='FPS only'\] \.hud-fps-only\{display:flex;flex-direction:row/);
});

test('saving Global settings reports whether the live overlay was updated',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/const saved=await call\('save_global_settings'/);
 assert.match(source,/notify\(saved\.liveNotice\|\|/);
});
test('Global numeric controls keep u8 and FPS values numeric on change',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/else draft\[key\]=t\.type==='checkbox'\?t\.checked:\['fps','scale','opacity'\]\.includes\(key\)\?Number\(t\.value\):t\.value;/);
});
test('global preview recalculates its HUD height when the preset or metrics change',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/function overlayHeight\(/);
 assert.match(source,/const height=overlayHeight\(preset,draft\.metrics\)/);
 assert.match(source,/hud\.style\.setProperty\('--hud-native-height',`\$\{overlayHeight\(\)\}px`\)/);
});
test('core navigation remains available and retired module writes are not exposed',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const commands=readFileSync(new URL('../src-tauri/src/main.rs',import.meta.url),'utf8');
 assert.match(source,/const visiblePages = \(\) => pages;/);
 assert.doesNotMatch(source,/set_module_enabled|moduleSwitch|data-preference=.["']?motion/);
 assert.doesNotMatch(commands,/runtime::set_module_enabled/);
});

test('dirty-state helpers distinguish saved settings from meaningful edits',()=>{
 const saved={overlay:true,preset:'Custom',layout:'Grid',palette:'Redunar',position:'Top left',scale:100,opacity:80,metrics:['FPS','GPU'],captureMetrics:true,replay:true,fps:60,quality:'Balanced',format:'MKV',storage:'10 GiB'};
 assert.equal(profileDraftChanged(structuredClone(saved),saved),false);
 assert.equal(profileDraftChanged({...saved,metrics:['GPU','FPS']},saved),false,'metric order is not a setting change');
 assert.equal(profileDraftChanged({...saved,opacity:70},saved),true);
 assert.equal(profileDraftChanged({...saved,layout:'Ribbon'},saved),true);
 assert.equal(profileDraftChanged({...saved,palette:'Mint'},saved),true);
 assert.equal(shortcutDraftChanged(['Shift+F8','F8'],['Shift+F8','F8']),false);
 assert.equal(shortcutDraftChanged(['Shift+F8','F9'],['Shift+F8','F8']),true);
 assert.equal(overrideDraftChanged({overlay:true},{overlay:true}),false);
 assert.equal(overrideDraftChanged({}, {overlay:true}),true);
 assert.equal(overrideDraftChanged({}, {}),false);
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/function updateDirtyActionButtons\(\)/);
 assert.match(source,/saveGlobal\.disabled=busy\|\|!currentChanged\|\|invalidCustom/);
 assert.match(source,/gameProfileChanged\(g\)\?'':'disabled'/);
});

test('Replay workspace aligns to content and limits the desktop clip rail to four cards',()=>{
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 assert.match(css,/\.replay-workspace\{align-items:start\}/);
 assert.match(css,/\.replay-save-row\{padding:18px 24px\}/);
 assert.match(css,/@media\(min-width:981px\)\{[\s\S]*\.clip-browser \.clip-list\{max-height:648px;overflow-y:auto/);
 assert.match(css,/\.clip-browser \.thumbnail\{height:72px\}/);
 assert.match(css,/@media\(max-width:980px\)\{[\s\S]*\.clip-browser \.clip-list\{max-height:none;overflow:visible/);
});
test('Replay opening keeps thumbnail work bounded and playback controls wait for media readiness',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 assert.match(source,/const prioritized=\[selectedClip,.*:visible\)/);
 assert.match(source,/\.slice\(0,4\);/);
 assert.match(source,/function scheduleClipThumbnails\(\)/);
 assert.match(source,/thumbnailLoadTimer=setTimeout\(\(\)=>\{thumbnailLoadTimer=null;if\(\['overview','replay'\]\.includes\(route\(\)\)\)loadClipThumbnails\(\);\},800\)/);
 assert.match(source,/let exporting = false, exportPollTimer = null, thumbnailLoadInFlight = false, thumbnailLoadTimer = null, videoReady = false/);
 assert.match(source,/videoReady=false;playRequested=false;updatePlayerUi\(\);/);
 assert.match(source,/toggle\.disabled=!videoReady\|\|exporting/);
 assert.match(source,/if\(!videoReady\|\|video\.readyState<1\)return;/);
 assert.match(source,/function togglePlayback\(\)/);
 assert.match(source,/document\.addEventListener\('pointerup',event=>\{[\s\S]*suppressPlayerToggleClickUntil=performance\.now\(\)\+500;[\s\S]*togglePlayback\(\);/);
 assert.match(source,/id="clip-play-toggle" class="icon-button player-toggle" type="button"/);
 assert.match(source,/const needsSeek=Math\.abs\(video\.currentTime-wanted\)>\.25;[\s\S]*try\{await video\.play\(\);break;\}/);
 assert.match(css,/\.player-controls \.icon-button\{color:#eee8ef;font-size:11px;width:42px;min-width:42px;height:34px/);
});
test('Native shell owns compact portable window chrome',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const html=readFileSync(new URL('../ui/index.html',import.meta.url),'utf8');
 const css=readFileSync(new URL('../ui/native.css',import.meta.url),'utf8');
 const nativeMain=readFileSync(new URL('../src-tauri/src/main.rs',import.meta.url),'utf8');
 const capability=JSON.parse(readFileSync(new URL('../src-tauri/capabilities/default.json',import.meta.url),'utf8'));
 assert.equal((html.match(/data-window-action=/g)||[]).length,3);
 assert.equal((html.match(/data-resize-direction=/g)||[]).length,8);
 assert.match(html,/class="window-titlebar" data-tauri-drag-region/);
 assert.match(html,/class="window-content"/);
 const config=JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json',import.meta.url),'utf8'));
 assert.equal(config.app.windows[0].decorations,false);
 assert.deepEqual(config.app.security.capabilities,['default']);
 assert.doesNotMatch(nativeMain,/window\.set_decorations\(false\)/);
 assert.doesNotMatch(nativeMain,/GDK_BACKEND|preferred_kde_gdk_backend|configure_window_backend/);
 assert.deepEqual(capability.windows,['main']);
 for(const permission of ['core:window:allow-close','core:window:allow-minimize','core:window:allow-start-dragging','core:window:allow-start-resize-dragging','core:window:allow-toggle-maximize'])assert.ok(capability.permissions.includes(permission),permission);
 assert.match(source,/getCurrentWindow/);
 assert.match(source,/appWindow\.startResizeDragging/);
 assert.match(source,/appWindow\.toggleMaximize/);
 assert.match(css,/\.window-titlebar \{[\s\S]*height: 28px/);
 assert.match(css,/\.window-content \{ position: fixed; inset: 28px 0 0;[\s\S]*overflow-y: auto/);
 assert.match(readFileSync(new URL('../ui/app.css',import.meta.url),'utf8'),/\.topbar\{[^}]*padding:0 16px 0 34px/);
});
test('Native window movement does not perform synchronous preference I/O',()=>{
 const nativeMain=readFileSync(new URL('../src-tauri/src/main.rs',import.meta.url),'utf8');
 assert.doesNotMatch(nativeMain,/WindowEvent::Resized\(_\) \|\| tauri::WindowEvent::Moved\(_\)/);
 assert.match(nativeMain,/if let tauri::WindowEvent::CloseRequested \{ api, \.\. \} = event \{/);
 assert.match(nativeMain,/Do not write preferences from every native move\/resize[\s\S]*persist_window_state\(window\);/);
 assert.match(nativeMain,/Do not write preferences from every native move\/resize/);
});
test('Tauri replay settings do not expose a total storage quota',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const replaySettings=source.slice(source.indexOf('function globalReplay()'),source.indexOf('function frameRateControl()',source.indexOf('function globalReplay()')));
 assert.doesNotMatch(replaySettings,/Saved clip storage/);
 assert.doesNotMatch(replaySettings,/data-global=["']storage/);
 assert.doesNotMatch(storageSummary({used_bytes:29.8e6,storage_limit_bytes:null}),/recording limit/);
});
test('Replay storage usage uses human-readable units without the verbose safety note',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 const html=storageSummary({used_bytes:29.8e6,available_bytes:137.4e9,total_bytes:512e9});
 assert.match(html,/Recordings.*29.8 MB/);
 assert.match(html,/137.4 GB <small>free/);
 assert.doesNotMatch(source,/Saves stop when the filesystem safety reserve would be crossed/);
 assert.doesNotMatch(html,/Saves stop when the filesystem safety reserve would be crossed/);
});
test('Discovery hides Redunar entries and disables games already in the catalog',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/alreadyImported/);
 assert.match(source,/Already in Redunar library/);
 assert.match(source,/candidate\.importable&&!existing/);
});
test('Runtime polling keeps expensive diagnostics and display probing off the one-second path',()=>{
 const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
 assert.match(source,/const slowRefresh=Date\.now\(\)-lastSlowRuntimeRefresh>=10000/);
 assert.match(source,/Promise\.allSettled\(\[call\('diagnostics_snapshot'\),call\('replay_display_capability'\)\]\)/);
 assert.match(source,/filter\(\(\[name\]\)=>name!==['"]reload-diagnostics['"]\)/);
 assert.match(source,/function pollDelay\(\)/);
 assert.match(source,/1000:2500/);
 assert.match(source,/if\(document\.hidden\)return 2000/);
});

test('clip metadata preserves observations and never invents game identity',()=>{
 const base={file_name:'long-real-recording-name.mkv',bytes:1024,modified_unix_ns:'1000000000'};
 const [unknown,known]=mapClips([base,{...base,game_name:'Local game',duration_seconds:29.5}]);
 assert.equal(unknown.game,null);assert.equal(unknown.duration,null);
 assert.equal(known.game,'Local game');assert.equal(known.duration,29.5);
 for(const duration_seconds of [0,-1,Infinity,'30'])assert.equal(mapClips([{...base,duration_seconds}])[0].duration,null);
});
