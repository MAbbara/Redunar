import './navigation-shell.mjs';
import { updateCard, updateSidebar } from './update-card.mjs';
import { dirtyGlobalTabs, historyFiltersMarkup, filterSessions, clipSortingToolbar } from './workspace-state.mjs';
import { canRetryShortcuts } from './shortcut-state.mjs';
import { diagnosticStatus } from './diagnostic-status.mjs';
import overlayPalettes from './overlay-palettes.json';
import { mergeGameDrafts } from './game-drafts.mjs';
import { startRuntime, latestRuntimeRefresh } from './runtime-polling.mjs';
import { loadClipPreview } from './clip-library.mjs';
import { replayStatusCopy } from './replay-menu-view.mjs';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { latestReplayRequest } from './replay-requests.mjs';
import { renderReplayFilmstrip } from './replay-filmstrip.mjs';
import { watchReplayLoading } from './replay-loading.mjs';
import { mapGames, mapClips, mapSessions, mapDefaults, profilePayload, overridePayload, measurement, visibleSession, profileDraftChanged, shortcutDraftChanged, overrideDraftChanged } from './native-data.mjs';
import { enhancePrecisionSelects, focusPrecisionSelect, closePrecisionSelect } from './precision-selects.js';
import { updatePrecisionSliders, updatePrecisionSlider } from './precision-sliders.js';
import { loadGameArtwork } from './game-artwork.js';
import { gameInstallation } from './game-installation.js';
import { steamSetupPanel, refreshSteamSetup } from './steam-setup.mjs';
import { historyMetrics, historyTimeline, historyDuration, historySelection, historySeries, historyScale, historyX, historyY, historyTime, historyKeyboardTime } from './history-timeline.mjs';
import { captureSummary, storageSummary, clipFiltersMarkup, clipFilterChips, filterClips, clipTitle, clipFilenameStem, renamedClipFilename, comparisonSelector, microphoneSelector } from './workspace-improvements.mjs';
const $ = (selector, root = document) => root.querySelector(selector);
const escape = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const paths = {
 overview:'M3 13h4l3-8 4 14 3-6h4', library:'M4 4h6v16H4zM14 4h6v16h-6z',
 global:'M4 7h16M4 17h16M8 4v6M16 14v6', replay:'M8 5H5v4M5 5a9 9 0 1 1-2 9M10 8l6 4-6 4z',
 history:'M12 8v5l3 2M3 10a9 9 0 1 1 1 7M3 4v6h6', settings:'M5 5h14v14H5zM9 9h6v6H9z',
 play:'M8 4l12 8-12 8z', pause:'M8 5v14M16 5v14', folder:'M3 6h7l2 3h9v11H3z',
 refresh:'M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1',
 arrow:'M5 12h14M14 7l5 5-5 5', check:'M5 12l4 4L19 6', search:'M20 20l-5-5M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0', download:'M12 3v12M7 10l5 5 5-5M4 17v4h16v-4', plus:'M12 4v16M4 12h16'
};
paths.trash='M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7';
paths.star='m12 3 2.8 5.7 6.3.9-4.5 4.4 1.1 6.2-5.7-3-5.7 3 1.1-6.2-4.5-4.4 6.3-.9Z';
const icon = (name, cls='') => `<svg class="icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${paths[name] || paths.global}"/></svg>`;
const clipFavoriteContent = favorite => `${icon('star','clip-favorite-icon')}<span>${favorite?'Favorited':'Favorite'}</span>`;
const pages = [['overview','Overview'],['library','Library'],['global','Global settings'],['replay','Instant Replay'],['history','History']];
let games = [], clips = [], sessions = [];
let savedGameOverrides = new Map();
const metricNames = ['FPS','Frame time','1% low','0.1% low','GPU','CPU','GPU temperature','CPU temperature'];

const presetMetrics = preset => preset==='Compact'?['FPS','Frame time','GPU','CPU','GPU temperature','CPU temperature']:preset==='Detailed'?[...metricNames]:preset==='FPS only'?['FPS']:[];
const effectiveMetrics = (preset,custom=[]) => preset==='Custom'?[...custom]:presetMetrics(preset||'Compact');
const initialDefaults = {overlay:null,preset:null,layout:null,palette:null,branding:true,position:null,scale:null,opacity:null,metrics:[],captureMetrics:null,replay:null,fps:null,quality:null,format:null,storage:null};
const durations = [15,30,60,120,180,300,600,900];
let selectedSaveDuration = null;
let defaults = structuredClone(initialDefaults);
let draft = structuredClone(defaults);
let shortcuts = Array(9).fill('');
let draftShortcuts = [...shortcuts];
let selectedGame = null, selectedClip = null, selectedSession = 0;
let overviewMetric = 'frame';
let historyMetric = 'fps', historyCompare = false, historyCursor = null, historyPeerId = '';
let clipFilters = {search:'',game:'all',date:'all',sort:'newest',favorites:false};
let historyFilters = {search:'',game:'all',date:'all'};
let lastTrashed = [], clipTrashFeedbackTimer = null;
let microphoneDevices = [];
const pendingFavorites = new Set();
let globalTab = 'overlay', libraryTab = 'profile';
let preferences = {tray:null,automaticUpdates:true,diagnosticLog:false,diagnosticLogPath:null,betaAccess:false};
let updateStatus = null, pendingUpdateAction = null;
let runningVersion=$('.build-label')?.textContent?.replace(/^Version\s+/,'')||'Current build';
let focusUpdatesOnRender=false;
let startupUpdateCheckStarted = false;
let toastTimer;
const native = Boolean(window.__TAURI_INTERNALS__);
const appWindow = native ? getCurrentWindow() : null;
const loaded = {catalog:false,clips:false,history:false,global:false,preferences:false,replayPreferences:false};
const errors = {};
let hardware = null, runtime = null, modules = null, diagnostics = null, displayCapability = null, activeSession = null, replayPreferences = null, storageStatus = null, globalRevision = null, savedShortcutMap = {}, busy = false;
let lastLoadedHistoryRevision = null;
let lastSlowRuntimeRefresh = 0;
let shortcutStatus = {state:'Inactive',message:null,last_action:null};
let playerDuration = 0, playerPosition = 0, pendingSeek = null, trimStart = 0, trimEnd = 0;
let stopFilmstrip = null;
let mediaEvents = null, thumbnailReloadRequested = false;
let timelineDrag = null, suppressTimelineClick = false, playbackFrame = null;
let pendingSeekTask = Promise.resolve(), cancelPendingSeek = null, seekGeneration = 0, mediaActionGeneration = 0, playRequested = false, suppressPlayerToggleClickUntil = 0;
let exporting = false, exportPollTimer = null, thumbnailLoadInFlight = false, thumbnailLoadTimer = null, videoReady = false;
const LIVE_TIMELINE_LIMIT = 240;
let liveTimeline = {game:'',revision:null,values:[],times:[]};
const call = (command,args={}) => native ? invoke(command,args) : Promise.reject(new Error('Open the native Redunar app to use the production backend.'));
const installation = gameInstallation(call, updateObservedElements);
const playbackRequests = latestReplayRequest(
 fileName=>call('clip_playback_path',{fileName}),
 ()=>call('cancel_clip_playback').catch(()=>{}),
);
const message = error => error instanceof Error ? error.message : String(error);
const diagnosticDisplayStatus=()=>{const status=diagnosticStatus(preferences.diagnosticLogStatus,preferences.diagnosticLog);return status==='Logging is off.'?'':status;};
const globalProfileChanged = () => profileDraftChanged(draft,defaults);
const globalShortcutsChanged = () => shortcutDraftChanged(draftShortcuts,shortcuts);
const gameProfileChanged = value => Boolean(value)&&overrideDraftChanged(value.overrides,savedGameOverrides.get(value.id));
const emptyPanel = (title,note) => `<section class="panel empty-panel"><span class="shortcut-icon">${icon('library')}</span><h2>${escape(title)}</h2><p>${escape(note)}</p></section>`;
const active = () => activeSession && (activeSession.can_end ?? !['Idle','Ended'].includes(activeSession.phase));
const showLibraryStartGuide = () => loaded.catalog && !errors.catalog && games.length===0;
const moduleText = value => {
 if(!value)return 'Status unavailable';
 if(value==='Enabled')return 'Ready';
 if(value==='DisabledByUser')return 'Disabled';
 if(value.startsWith('UnavailableOnSystem'))return 'Unavailable';
 if(value.startsWith('PlannedUnavailable'))return 'Planned';
 return value;
};
const moduleClass = value => value==='Enabled'?'green':!value||value==='DisabledByUser'?'':'red';
const shortcutClass = value => value==='Active'?'green':value==='Starting'?'':'red';
const shortcutText = value => value==='Active'?'Active':value==='Starting'?'Starting…':value||'Inactive';
function shortcutBadge(){return `<span class="pill ${shortcutClass(shortcutStatus?.state)}" data-shortcut-status>${escape(shortcutText(shortcutStatus?.state))}</span>`;}
function moduleBadge(key,label='') {
 return `<span class="pill ${moduleClass(modules?.[key])}" data-module-status="${key}" data-module-label="${label}">${escape((label?label+' · ':'')+moduleText(modules?.[key]))}</span>`;
}
const workspace = $('#workspace');
const scrollRoot = $('.window-content');
const clip = () => clips.find(c=>c.id===selectedClip);
const game = () => games.find(g=>g.id===selectedGame);
const route = () => location.hash.slice(1) || 'overview';
const visiblePages = () => pages;
const time = seconds => seconds == null || !Number.isFinite(seconds) ? '—' : `${String(Math.floor(seconds/60)).padStart(2,'0')}:${String(Math.floor(seconds%60)).padStart(2,'0')}`;
const preciseTime = seconds => seconds == null || !Number.isFinite(seconds) ? '—' : `${String(Math.floor(seconds/60)).padStart(2,'0')}:${(seconds%60).toFixed(1).padStart(4,'0')}`;
const pill = (text, cls='') => `<span class="pill ${cls}">${text}</span>`;
const button = (text, action, primary=false, extra='') => `<button class="button ${primary?'primary':''}" data-action="${action}" ${extra}>${text}</button>`;
function heading(title, description, action='') {
 return `<div class="page-heading"><div><h1>${title}</h1><p>${description}</p></div>${action}</div>`;
}
function stat(label,value,unit='',note='',id='') {
 return `<div class="stat"><span>${label}</span><strong${id?` id="${id}"`:""}>${value}<small>${unit}</small></strong>${note?`<p>${note}</p>`:''}</div>`;
}
function chart(values=[], unit='ms', ariaLabel='recorded frame intervals') {
 if(!values.length)return `<div class="chart empty-chart"><p>No recorded frame intervals available.</p></div>`;
 const ceiling=Math.max(5,Math.ceil(Math.max(...values)/5)*5);
 const displayed=aggregateTimelinePoints(values.map((value,index)=>({x:index,value})),Math.max(1,values.length-1)).map(point=>point.value);
 const points=displayed.map((v,i)=>`${i/Math.max(1,displayed.length-1)*900},${210-v/ceiling*200}`).join(' ');
 const live=ariaLabel==='recent frame-time timeline';
 return `<div class="chart overview-timeline-chart${live?' live-timeline-chart':''}"><div class="y-axis">${[1,.75,.5,.25].map(n=>`<span>${(ceiling*n).toFixed(1)} ${unit}</span>`).join('')}</div><svg viewBox="0 0 900 220" preserveAspectRatio="none" role="img" aria-label="${values.length} ${ariaLabel}"><defs><linearGradient id="overview-area-gradient" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f15c6c" stop-opacity=".2"/><stop offset="1" stop-color="#f15c6c" stop-opacity="0"/></linearGradient></defs>${[10,60,110,160,210].map(y=>`<path d="M0 ${y}H900" class="gridline"/>`).join('')}<polygon points="0,210 ${points} 900,210" class="overview-area"></polygon><polyline points="${points}" class="trace"/></svg></div><div class="chart-axis"><span>${live?'Older':'Sample 1'}</span><span>${values.length} retained samples${live?` · ${time((liveTimeline.times.at(-1)-liveTimeline.times[0])/1000)} sample span`:''}</span><span>${live?'Latest sample':`Sample ${values.length}`}</span></div>`;
}

function mark(g) {return `<span class="game-mark" data-poster="${escape(g.id)}">${icon('library')}</span>`;}
function switchControl(key,label,checked,scope='global',disabled=false) {
 return `<label class="switch"><input type="checkbox" data-${scope}="${key}" aria-label="${label}" ${checked?'checked':''} ${disabled?'disabled':''}><span></span></label>`;
}
function selectControl(key,label,values,value,scope='global') {
 return `<select aria-label="${label}" data-${scope}="${key}">${values.map(v=>`<option ${String(v)===String(value)?'selected':''}>${escape(v)}</option>`).join('')}</select>`;
}
function field(label,description,control) {
 return `<div class="field"><div><label>${label}</label>${description?`<p>${description}</p>`:''}</div>${control}</div>`;
}
function overlayHeight(preset=draft.preset||'Compact',metrics=draft.metrics) {
 const shown=effectiveMetrics(preset,metrics);
 const has=name=>shown.includes(name);
 const frame=has('FPS')||has('Frame time');
 const lows=has('1% low')||has('0.1% low');
 const hardware=has('GPU')||has('CPU')||has('GPU temperature')||has('CPU temperature');
 return preset==='FPS only'?24:25+(frame?38:0)+(lows?26:0)+(hardware?26:0);
}
function ribbonWidth(preset=draft.preset||'Compact',metrics=draft.metrics) {
 return (draft.branding===false?0:94)+effectiveMetrics(preset,metrics).length*89;
}
function overlayPreview() {
 const preset=draft.preset||'Compact';
 const height=overlayHeight(preset,draft.metrics);
 const layout=draft.layout||'Grid';
 const palette=overlayPalettes[draft.palette]||overlayPalettes.Redunar;
 return `<div class="overlay-scene vulkan-reference"><div id="hud" class="hud" data-position="${draft.position}" data-preset="${escape(preset)}" data-layout="${escape(layout)}" style="--hud-scale:${draft.scale/100};--hud-preview-fit:${layout==='Ribbon'?.78:1};--hud-ribbon-width:${ribbonWidth(preset,draft.metrics)}px;--hud-opacity:${draft.opacity/100};--hud-native-height:${height}px;--hud-accent:${palette[0]};--hud-panel:${palette[1]};--hud-text:${palette[2]};--hud-muted:${palette[3]};--hud-border:${palette[4]}">${hudContent()}</div></div><p class="overlay-preview-note">In-game overlay layout · <span id="overlay-preview-layout">${escape(layout)}</span> · <span id="overlay-preview-palette">${escape(draft.palette||'Redunar')}</span> palette · <span id="overlay-preview-scale">${draft.scale}</span>% scale</p>`;
}
function hudContent() {
 const preset=draft.preset||'Compact';
 if(preset==='FPS only')return `<div class="hud-fps-only"><span>FPS</span><b>144</b></div>`;
 const shown=effectiveMetrics(preset,draft.metrics);
 const has=name=>shown.includes(name);
 const frame=has('FPS')||has('Frame time');
 const lows=has('1% low')||has('0.1% low');
 const hardware=has('GPU')||has('CPU')||has('GPU temperature')||has('CPU temperature');
 const items=[
  has('FPS')&&['FPS','144 FPS'],has('Frame time')&&['FRAME TIME','6.9 MS'],has('1% low')&&['1% LOW','118 FPS'],has('0.1% low')&&['0.1% LOW','96 FPS'],
  has('GPU')&&['GPU LOAD','91%'],has('GPU temperature')&&['GPU TEMP','68°C'],has('CPU')&&['CPU LOAD','38%'],has('CPU temperature')&&['CPU TEMP','62°C']
 ].filter(Boolean);
 if(draft.layout==='Ribbon')return `${draft.branding===false?'':'<div class="hud-layout-brand"><span>REDUNAR</span></div>'}${items.map(([label,value])=>`<div class="hud-ribbon-metric"><small>${label}</small><b>${value}</b></div>`).join('')}`;
 if(draft.layout==='Telemetry')return `<div class="hud-telemetry-head">${draft.branding===false?'':'<span>REDUNAR</span>'}<small>FRAME METRICS</small></div>${items.map(([label,value])=>`<div class="hud-telemetry-row"><span>${label}</span><b>${value}</b></div>`).join('')}`;
 const sections=[];
 if(frame)sections.push(`<div class="hud-native-frame">${has('FPS')?'<b class="hud-native-value hud-native-fps-value">144</b><span class="hud-native-label hud-native-fps-label">FPS</span>':''}${has('Frame time')?'<b class="hud-native-value hud-native-frame-time-value">6.9</b><span class="hud-native-label hud-native-frame-time-label">MS</span>':''}</div>`);
 if(lows)sections.push(`<div class="hud-native-row hud-native-lows">${has('1% low')?'<span class="hud-native-left">1% LOW&nbsp;&nbsp;118</span>':''}${has('0.1% low')?'<span class="hud-native-right">0.1% LOW&nbsp;&nbsp;96</span>':''}</div>`);
 if(hardware)sections.push(`<div class="hud-native-row hud-native-hardware">${has('CPU')||has('CPU temperature')?`<span class="hud-native-left">CPU${has('CPU')?' 38%':''}${has('CPU temperature')?(has('CPU')?' · 62°C':' 62°C'):''}</span>`:''}${has('GPU')||has('GPU temperature')?`<span class="hud-native-right">GPU${has('GPU')?' 91%':''}${has('GPU temperature')?(has('GPU')?' · 68°C':' 68°C'):''}</span>`:''}</div>`);
 return `${draft.branding===false?'':'<div class="hud-native-header">REDUNAR</div>'}${sections.join('')}`;
}
function replaySaveActions() {
 return `<div class="heading-actions replay-save-actions"><label class="replay-duration-label">Save last <select id="save-duration" aria-label="Replay duration">${durations.map(d=>`<option value="${d}" ${d===(selectedSaveDuration??Number(replayPreferences?.initial_save_duration_seconds||30))?'selected':''}>${d<60?d+' seconds':d/60+(d===60?' minute':' minutes')}</option>`).join('')}</select></label>${button(`${icon('plus')} Save replay <span class="replay-shortcut-slot" hidden><kbd id="save-replay-shortcut" aria-hidden="true" hidden></kbd></span>`,'save-replay',true,`aria-label="Save replay" ${!runtime?.can_save?'disabled':''}`)}</div>`;
}
function replayCaptureBar(page) {
 return `<div class="replay-capture-bar ${page==='overview'?'overview-capture-bar':''}"><div id="session-capture-summary">${captureSummary(runtime)}</div>${replaySaveActions()}</div>`;
}
function replay() {
 const c=clip();
 const usedBytes=storageStatus?.used_bytes??clips.reduce((sum,item)=>sum+item.bytes,0);

 return `<div class="page-heading replay-page-heading"><h1>Instant Replay</h1><a class="button replay-settings-link" href="#global" data-global-tab="replay">${icon('global')} Capture settings</a></div>`+`
 ${replayCaptureBar('replay')}<div class="replay-workspace"><section class="editor"><div id="replay-selection">${replaySelection(c)}</div>
</section><aside class="clip-browser"><div class="section-heading"><h3>Saved clips</h3><div class="heading-actions clip-library-actions">${button(icon('folder'),'folder',false,'aria-label="Clip folder" title="Clip folder"')}${button(icon('refresh'),'reload-clips',false,'aria-label="Refresh clips" title="Refresh clips"')}${button(icon('trash'),'view-clip-trash',false,'aria-label="Trash bin" title="Trash bin"')}</div></div><label class="search">${icon('search')}<input type="search" id="clip-search" placeholder="Find a clip" aria-label="Find a clip" value="${escape(clipFilters.search)}"></label>${clipFiltersMarkup(clips,clipFilters)}<div id="clip-organizing-controls">${clipSortingControlsMarkup()}</div><div class="clip-trash-feedback" id="clip-trash-feedback" ${lastTrashed.length?'':'hidden'}><span>${lastTrashed.length} ${lastTrashed.length===1?'clip':'clips'} moved to trash bin</span><button class="text-button" data-action="undo-clip-trash">Undo</button></div><div class="clip-list">${clipRows()}</div>${storageSummary({...storageStatus,used_bytes:usedBytes})}</aside></div>`;
}

function replaySelection(c) {
 if(!c)return emptyPanel(errors.clips?'Clips unavailable':'Your next highlight belongs here.',errors.clips||'Saved Redunar clips will appear here. Nothing is uploaded.');
 return `<div class="clip-title-tools"><div class="clip-name"><strong data-selected-clip-title>${escape(clipTitle(c))}</strong><small>${escape(c.date)} · ${escape(c.size)} · ${escape(c.file_name.split('.').pop().toUpperCase())}</small></div><div class="heading-actions clip-edit-actions">${button('Edit filename','edit-clip-filename')}${button(clipFavoriteContent(c.favorite),'favorite-selected-clip',false,`aria-pressed="${Boolean(c.favorite)}"`)}<details class="clip-action-menu"><summary class="button" aria-label="Clip actions" title="Clip actions">⋯</summary><div class="replay-more-menu">${button('Open externally','open-clip-external')}${button('Move to trash bin','delete-selected-clip')}</div></details></div></div><div class="player native-player"><div class="native-video-stage"><video id="clip-video" ${c.thumbnail?`poster="${c.thumbnail}"`:""} preload="auto" aria-label="${escape(clipTitle(c))}"></video><div class="native-video-specs" id="clip-video-specs"></div><button class="center-play" id="center-play" data-player="toggle" aria-label="Play clip" hidden>${icon('play')}</button><div class="media-state-overlay" id="media-state-overlay" role="status"><div><strong id="media-state-title">Preparing the clip…</strong><p id="media-state-detail">Getting your local capture ready for playback.</p><div class="heading-actions"><button class="button" id="retry-clip" data-action="retry-clip" hidden>Retry playback</button>${button('Open externally','open-clip-external')}</div></div></div><div class="native-video-caption"><strong>${escape(c.game_name||'Game not recorded')}</strong></div></div>
 <div class="player-controls"><button id="clip-play-toggle" class="icon-button player-toggle" type="button" data-player="toggle" aria-label="Play clip" title="Play clip">${icon('play')}</button><button class="icon-button transport-jump" type="button" data-player="back" aria-label="Back five seconds">−5</button><button class="icon-button transport-jump" type="button" data-player="forward" aria-label="Forward five seconds">+5</button><input id="player-seek" type="range" min="0" max="0" value="0" step="0.05" aria-label="Seek in selected clip" disabled><span class="timecode" id="clip-time">00:00 / 00:00</span></div>
 </div>
 <section class="replay-clip-editor" aria-label="Clip editor">
 <div class="clip-time-ruler" id="clip-time-ruler" aria-hidden="true">${Array(7).fill('<span>—</span>').join('')}</div>
 <div class="clip-timeline" id="clip-timeline" data-timeline="seek" role="slider" tabindex="0" aria-label="Playback position" aria-disabled="true" aria-valuemin="0" aria-valuemax="1" aria-valuenow="0"><canvas class="clip-filmstrip" id="clip-filmstrip" aria-hidden="true"></canvas><div class="clip-filmstrip-shade"></div><div class="timeline-selection" id="trim-selection"></div><button class="timeline-handle start" id="trim-start-bar" data-timeline="start" role="slider" aria-label="Trim start" aria-valuemin="0" aria-valuemax="1" aria-valuenow="0" disabled></button><span class="timeline-playhead" id="clip-playhead" aria-hidden="true"></span><button class="timeline-handle end" id="trim-end-bar" data-timeline="end" role="slider" aria-label="Trim end" aria-valuemin="0" aria-valuemax="1" aria-valuenow="1" disabled></button></div>
 <div class="clip-editor-footer"><span class="trim-time">In <output id="trim-start-value">—</output></span><span class="trim-time">Out <output id="trim-end-value">—</output></span><span class="trim-time selection-time">Selected <output id="clip-selected-duration">—</output></span><button class="text-button" data-action="reset-trim" disabled>Reset</button>${button(`${icon('download')} Export selection`,'export',false,'id="export-clip-button" disabled')}</div><p class="clip-preview-status" id="clip-preview-status">Loading frame previews…</p></section>`;
}

function clipRows(filter=clipFilters.search) {
 clipFilters.search=filter;
 const filtered=filterClips(clips,clipFilters);
 return filtered.map(c=>`<div class="clip-card ${selectedClip===c.id?'selected':''}"><button type="button" class="clip-star" data-favorite-clip="${escape(c.id)}" aria-label="${c.favorite?'Remove from':'Add to'} favorites: ${escape(clipTitle(c))}" aria-pressed="${Boolean(c.favorite)}">${icon('star','clip-favorite-icon')}</button><button class="clip-select" data-clip="${escape(c.id)}" aria-pressed="${selectedClip===c.id}"><span class="thumbnail native-thumbnail">${c.thumbnail?`<img src="${c.thumbnail}" alt="">`:icon('play')}<span data-clip-duration>${time(c.duration)}</span></span><strong class="clip-title">${escape(clipTitle(c))}</strong><span class="clip-game">${escape(c.game_name||'Game not recorded')}</span><small>${escape(c.date)}<span>${escape(c.size)}</span></small></button></div>`).join('')||`<div class="clip-empty"><strong>${clips.length?'No matching clips':'No saved clips yet'}</strong>${clips.length?button('Clear filters','clear-clip-filters'):'<p>Save recent gameplay during a supported session. Captures stay on this device.</p>'}</div>`;
}
function updateClipBrowserControls(){
 const count=filterClips(clips,clipFilters).length;
 const results=$('#clip-result-count');if(results)results.textContent=`${count} ${count===1?'clip':'clips'}`;
 document.querySelectorAll('[data-clip-scope]').forEach(button=>button.setAttribute('aria-pressed',String((button.dataset.clipScope==='favorites')===clipFilters.favorites)));
 const badge=$('[data-filter-count]');if(badge){const active=Number(clipFilters.game!=='all')+Number(clipFilters.date!=='all');badge.textContent=String(active);badge.hidden=active===0;}
 const chips=$('.clip-active-filters');if(chips)chips.innerHTML=clipFilterChips(clipFilters);
}
function clipSortingControlsMarkup() {
 return clipSortingToolbar(filterClips(clips,clipFilters).length,clipFilters.sort);
}
function refreshClipSortingControls() {
 const controls=$('#clip-organizing-controls');
 if(controls){controls.innerHTML=clipSortingControlsMarkup();enhancePrecisionSelects(controls);}
}
function refreshClipTrashFeedback(){
 const feedback=$('#clip-trash-feedback');
 if(feedback){feedback.hidden=!lastTrashed.length;feedback.querySelector('span').textContent=`${lastTrashed.length} ${lastTrashed.length===1?'clip':'clips'} moved to trash bin`;}
}
function clearClipTrashFeedback(){clearTimeout(clipTrashFeedbackTimer);clipTrashFeedbackTimer=null;lastTrashed=[];refreshClipTrashFeedback();}
function showClipTrashFeedback(fileNames){
 clearTimeout(clipTrashFeedbackTimer);lastTrashed=fileNames;
 // Only the Undo notice expires; recordings remain recoverable in the trash bin.
 clipTrashFeedbackTimer=setTimeout(()=>{lastTrashed=[];clipTrashFeedbackTimer=null;refreshClipTrashFeedback();},10000);
}
function refreshClipRail(){
 refreshClipSortingControls();refreshClipTrashFeedback();
 const rail=$('.clip-list');if(rail){rail.innerHTML=clipRows();updateClipBrowserControls();scheduleClipThumbnails();}
}
function syncClipFilterFields(){document.querySelectorAll('[data-clip-filter]').forEach(field=>{field.value=clipFilters[field.dataset.clipFilter];});enhancePrecisionSelects(workspace);}

async function saveClipDetails(item,changes){
 if(pendingFavorites.has(item.id)||busy)return;
 const key=item.id;pendingFavorites.add(key);
 try {
 const result=await call('set_clip_details',{fileName:item.file_name,...changes});
 Object.assign(item,result);refreshClipRail();
 const title=$('[data-selected-clip-title]');if(title&&selectedClip===item.id)title.textContent=clipTitle(item);
 const video=$('#clip-video');if(video&&selectedClip===item.id)video.setAttribute('aria-label',clipTitle(item));const favorite=$('[data-action="favorite-selected-clip"]');if(favorite){favorite.innerHTML=clipFavoriteContent(clip()?.favorite);favorite.setAttribute('aria-pressed',String(Boolean(clip()?.favorite)));}
 } finally {pendingFavorites.delete(key);}
}

function overview() {
 const meter=(name)=>`<div class="hardware"><div class="hardware-heading"><span>${name}</span><strong><b id="${name}-load">—</b><small>%</small></strong></div><div class="meter"><i id="${name}-meter"></i></div><p id="${name}-model">Waiting for the monitor</p><div class="temperature-heading"><span>Temperature</span><b id="${name}-temp">—</b></div><div class="meter temperature-meter" title="Display scale: 0–100°C"><i id="${name}-temperature-meter"></i></div></div>`;
 return heading('Overview','Current session, frame pacing, and system measurements.')+`
 <section class="session-banner status-footer"><div class="session-main"><div class="session-game"><span class="game-mark">${icon('play')}</span><div><small id="session-phase">Connecting</small><h2 id="session-name">No active game</h2></div></div><div class="session-end"><div class="session-time"><small>Time played</small><time id="session-timer" aria-label="Session elapsed time">—</time></div>${button('End session','end-session',true,active()?'':'hidden')}</div></div><div class="session-footer"><span id="session-profile">Profile unavailable</span><span id="session-captures">— captures saved</span><span id="session-features">Waiting for session status</span></div></section>
 <div class="session-feedback"><p id="session-note"></p></div>${replayCaptureBar('overview')}
 <div id="library-start-guide" class="library-start-guide" ${showLibraryStartGuide()?'':'hidden'}><div><strong>Add your first game</strong><p>Add or scan games in Library to get started.</p></div><a class="button" href="#library">Open Library</a></div>
 <div class="overview-grid"><section class="panel pacing-panel"><div class="section-heading"><div><h2>Frame pacing</h2><p>Recent live samples. Open History for the completed session.</p></div><div class="history-view-toggle overview-view-toggle">${[['frame','Frame time'],['fps','FPS']].map(([id,label])=>`<button data-overview-view="${id}" class="${overviewMetric===id?'active':''}" aria-pressed="${overviewMetric===id}">${label}</button>`).join('')}</div></div><p class="live-status" id="live-phase">Awaiting telemetry</p><div class="telemetry-row">${stat('Average frame rate','—','FPS','','live-fps')}${stat('1% low','—','FPS','','live-low')}${stat('0.1% low','—','FPS','','live-lowest')}${stat('Average frame time','—','ms','','live-frame-time')}</div><div id="live-chart">${chart()}</div><div class="panel-footer"><span><i class="legend"></i><span id="overview-chart-label">Frame time</span></span><a href="#history">View session history ${icon('arrow')}</a></div></section>
 <section class="panel system-panel"><h2>System metrics</h2><p>Live hardware readings</p>${meter('GPU')}${meter('CPU')}${['ram','vram'].map(name=>`<div class="memory-block"><div><span>${name.toUpperCase()}</span><b id="${name}-used">—</b></div><div class="meter"><i id="${name}-meter"></i></div><p id="${name}-total">Capacity unavailable</p><p id="${name}-available"></p></div>`).join('')}<p class="temperature-scale">Temperature scale: 0–100°C</p></section></div><div class="overview-links"><a href="#history">Session history ${icon('arrow')}</a><a href="#global">Configure global defaults ${icon('arrow')}</a></div>
 <section class="recent-captures"><div class="section-heading"><div><h2>Recent captures</h2><p id="overview-replay-status">Waiting for replay status</p></div><a href="#replay">View all ${icon('arrow')}</a></div><div class="recent-capture-grid">${clips.slice(0,3).map(c=>`<a href="#replay" data-recent-clip="${escape(c.id)}"><span class="recent-thumbnail">${c.thumbnail?`<img src="${c.thumbnail}" alt="">`:icon('play')}<span data-clip-duration>${time(c.duration)}</span></span><span><strong>${escape(c.game_name||'Game not recorded')}</strong><small class="recent-clip-file" title="${escape(c.file_name)}">${escape(c.file_name)}</small><small>${escape(c.date)} · ${escape(c.size)}</small></span></a>`).join('')||'<p class="empty">Saved replays will appear here.</p>'}</div></section>`;
}

function libraryRows(filter='') {
 return games.filter(g=>g.name.toLowerCase().includes(filter.toLowerCase())).map(g=>`<button class="library-game ${g.id===selectedGame?'selected':''}" data-game="${g.id}" aria-pressed="${g.id===selectedGame}">${mark(g)}<span><strong>${escape(g.name)}</strong><small>${escape(g.launcher)}</small><small>${Object.keys(g.overrides).length?'Custom settings':'Global defaults'}</small></span></button>`).join('') || '<p class="empty">No matching games.</p>';
}
function library() {
 const g=game();
 const header=heading('Library','Games, launch options, and individual profiles.',`<div class="heading-actions library-heading-actions">${button('<span>Scan installed games</span>','scan-games')}${button('<span>Reload library</span>','reload-games')}${button(`${icon('plus')}<span>Add game</span>`,'add-game',true)}</div>`);
 if(!g)return header+`<section class="panel empty-panel"><span class="shortcut-icon">${icon('library')}</span><h2>${errors.catalog?'Library unavailable':'Your game library starts here.'}</h2><p>${escape(errors.catalog||'Scan for installed games, or add a local game yourself. Settings stay on this device.')}</p><div class="heading-actions">${button('Scan installed games','scan-games',true)}${button('Add a game','add-game')}</div></section>`;
 return header+
 `<div class="library-workspace"><aside class="game-catalog"><label class="search">${icon('search')}<input id="game-search" type="search" placeholder="Find a game" aria-label="Find a game"></label><div class="catalog-label">${games.length} local games</div><div id="library-list">${libraryRows()}</div><div class="catalog-note">${icon('folder')}<p>Detected locally or added by you.</p></div></aside><section class="game-detail"><div class="game-cover ${g.color}"><div class="game-cover-banner" data-banner="${escape(g.id)}"></div><div class="cover-art" aria-hidden="true">${icon('library')}</div><div>${pill(escape(g.launcher))}<h2>${escape(g.name)}</h2><p>Local game profile</p></div><div class="game-cover-actions"><div class="game-launch-action">${button(`${icon('play')} Launch game`,'launch-game',true,'disabled aria-describedby="game-installation-note"')}<small id="game-installation-note" aria-live="polite"></small></div><details class="game-more"><summary class="button">More <span aria-hidden="true">⌄</span></summary><div>${button('Edit launch settings','edit-launch')}${button('Remove game','remove-game')}</div></details></div></div>${g.steam_app_id?steamSetupPanel():''}<div class="tabbar">${[['profile','Game settings'],['match','Launch matching']].map(([id,label])=>`<button data-library-tab="${id}" class="${libraryTab===id?'active':''}" aria-pressed="${libraryTab===id}">${label}</button>`).join('')}</div>${libraryTab==='match'?`<div class="detail-body"><h3>Identify this game</h3><p>Exact local matches connect a game to its settings.</p>${g.steam_app_id?field('Steam App ID','Identifies this game when launched from Steam',`<code>${escape(g.steam_app_id)}</code>`):field('Executable','Exact local match',`<code class="launch-match-value">${escape(g.exe)}</code>`)}${g.steam_app_id?field('Launch executable','Starts Steam; the App ID identifies the game',`<code>${escape(g.exe)}</code>`):''}${field('Launcher','Optional metadata',`<span>${escape(g.launcher)}</span>`)}<div class="info-note">Ambiguous matches must be reviewed before a profile can activate.</div></div>`:libraryProfile(g)}</section></div>${gameSaveBar(g)}`;
}
function gameSaveBar(g) {
 return `<div class="game-save-bar change-save-bar" ${gameProfileChanged(g)?'':'hidden'}><span id="game-save-state">${gameProfileChanged(g)?'Unsaved changes':'All changes saved'}</span><div class="heading-actions">${button('Discard changes','discard-game',false,gameProfileChanged(g)?'':'disabled')}${button('Save changes','save-game',true,gameProfileChanged(g)?'':'disabled')}</div></div>`;
}
function libraryProfile(g) {
 const rows=[['overlay','Show in-game overlay',['On','Off']],['captureMetrics','Frame metrics',['On','Off']]];
 const visibleCustomValues=Object.keys(g.overrides).filter(key=>['overlay','captureMetrics'].includes(key)).length;
 return `<div class="detail-body"><div class="inheritance-banner">${icon('global')}<div><strong>${visibleCustomValues?`${visibleCustomValues} custom setting${visibleCustomValues===1?'':'s'}`:'Uses global settings'}</strong><p>${visibleCustomValues?'Other settings continue to follow your global defaults.':'Changes to global defaults also apply to this game.'}</p></div></div><div class="readiness-strip"><span>Available on this system</span>${[['frame_metrics','Frame metrics'],['in_game_overlay','In-game metrics'],['instant_replay','Instant Replay']].map(([key,label])=>`<span>${label} <b data-readiness-status="${key}" data-ready="${modules?.[key]==='Enabled'}">${escape(moduleText(modules?.[key]))}</b></span>`).join('')}</div>${rows.map(([key,label,opts])=>{
 const inherited=!(key in g.overrides),value=inherited?defaults[key]:g.overrides[key];
 const display=typeof value==='boolean'?(value?'On':'Off'):value;
 return field(label+(inherited?'':' <span class="custom-setting">Custom</span>'),key==='overlay'?(inherited?'Inherited from Global settings · Updates this game when saved':'Updates this game when saved, including while running'):(inherited?'Inherited from Global settings':'Applies only to this game'),`<div class="override-control"><select data-override="${key}" aria-label="${label}"><option value="inherit" ${inherited?'selected':''}>Global · ${typeof defaults[key]==='boolean'?(defaults[key]?'On':'Off'):(defaults[key]??'Unavailable')}</option>${opts.map(v=>`<option value="${v}" ${!inherited&&String(v)===String(display)?'selected':''}>${v}</option>`).join('')}</select></div>`);
 }).join('')}<div class="game-profile-footer"><button class="text-button" data-action="reset-game" ${visibleCustomValues?'':'disabled'}>Reset to global settings</button></div><p class="small-note">Overlay appearance, capture frame rate, quality, file format, and shortcuts use Global settings.</p></div>`;
}
function globalSettings() {
 if(!loaded.global)return heading('Global settings','Your starting point for every game.',button('Reload defaults','reload-global'))+emptyPanel('Saved defaults unavailable',errors.global||'Connecting to the production backend…');
 const emptyCustom=globalTab==='overlay'&&draft.preset==='Custom'&&!draft.metrics.length;
 const profileChanged=globalProfileChanged(),shortcutsChanged=globalShortcutsChanged();
 const anyChanged=profileChanged||shortcutsChanged;
 const currentChanged=globalTab==='shortcuts'?shortcutsChanged:profileChanged;
 const saveBar=`<div class="settings-save-bar change-save-bar" ${anyChanged?'':'hidden'}><span id="global-save-feedback" role="status"></span><div class="heading-actions">${button('Discard changes','discard-global',false,anyChanged?'':'disabled')}${button(globalTab==='shortcuts'?'Save shortcut assignments':'Save changes','save-global',true,(!currentChanged||emptyCustom)?'disabled':'')}</div></div>`;
 return heading('Global settings','Default metrics and replay settings for every game.')+
 `<div class="tabbar global-tabs">${[['overlay','In-game metrics'],['replay','Instant replay'],['shortcuts','Keyboard shortcuts']].map(([id,label])=>`<button data-global-tab="${id}" class="${globalTab===id?'active':''}" aria-pressed="${globalTab===id}">${label}<span class="unsaved-tab-marker" aria-hidden="true" hidden>●</span></button>`).join('')}</div>${globalTab==='overlay'?globalOverlay():globalTab==='replay'?globalReplay():globalShortcuts()}${saveBar}`;
}
function globalOverlay() {
 const swatches=Object.entries(overlayPalettes).map(([name,colors])=>`<button type="button" class="overlay-palette" data-palette="${name}" aria-label="${name} palette" aria-pressed="${draft.palette===name}" style="--swatch:${colors[0]}"></button>`).join('');
 return `<div class="global-grid"><section class="overlay-preview-panel">${overlayPreview()}<div class="overlay-option-heading"><h3>Layout</h3></div><div class="layout-grid">${['Grid','Ribbon','Telemetry'].map(name=>`<button data-layout="${name}" aria-pressed="${draft.layout===name}"><strong>${name}</strong><small>${name==='Grid'?'Grouped measurements':name==='Ribbon'?'Horizontal strip':'Stacked measurements'}</small></button>`).join('')}</div><div class="overlay-option-heading"><h3>Color palette</h3><span id="overlay-palette-name">${escape(draft.palette||'Redunar')}</span></div><div class="palette-grid" role="group" aria-label="Overlay palette">${swatches}</div><div class="overlay-option-heading"><h3>Information preset</h3></div><div class="preset-grid">${['FPS only','Compact','Detailed','Custom'].map(name=>`<button data-preset="${name}" aria-pressed="${draft.preset===name}"><span>${name==='Custom'?'+':'144'}${name!=='FPS only'?'<small>Frame metrics</small>':''}</span><strong>${name}</strong></button>`).join('')}</div><h2 class="metrics-title">Displayed metrics</h2><p class="metric-hint">${draft.preset==='Custom'?'Custom selection · choose the metrics shown in any layout.':'Preset controlled · choose Custom to edit individual metrics.'}</p><p class="metric-validation" id="metric-validation" role="status"></p><div class="metric-toggles">${metricNames.map(name=>`<label><input type="checkbox" data-metric="${name}" ${effectiveMetrics(draft.preset||'Compact',draft.metrics).includes(name)?'checked':''} ${draft.preset==='Custom'?'':'disabled'}><span>${name}</span></label>`).join('')}</div></section><section class="panel controls-panel"><h2>Overlay appearance</h2>${field('Collect frame metrics','Record frame-time data for future sessions',switchControl('captureMetrics','Frame metrics',draft.captureMetrics))}${field('Show in-game overlay'+(active()?' <span class="auto-saved-label">Auto-saved</span>':''),active()?'Shows or hides immediately; keeps the overlay available. Per-game overrides still apply.':'Hides only the metrics display; you can show it again during a game.',switchControl('overlay','Show in-game overlay',draft.overlay))}${field('Show Redunar branding','Shows the Redunar label in the in-game metrics overlay',switchControl('branding','Show Redunar branding',draft.branding!==false))}${field('Position','',selectControl('position','Position',['Top left','Top right','Bottom left','Bottom right'],draft.position))}${['scale','opacity'].map(key=>field(key==='scale'?'Scale':'Opacity','',`<div class="range-control"><output id="${key}-value">${draft[key]}%</output><input type="range" data-global="${key}" aria-label="${key==='scale'?'Scale':'Opacity'}" min="${key==='scale'?50:0}" max="${key==='scale'?200:100}" step="${key==='scale'?5:1}" value="${draft[key]}"></div>`)).join('')}</section></div>`;
}
function globalReplay() {
 const folder=replayPreferences?.resolved_directory||'Loading replay folder…';
 return `<section class="panel controls-panel"><h2>Capture defaults</h2>${field('Instant replay','Buffers automatically for supported games launched through Redunar or configured Steam launch options. Clear shortcuts to prevent keyboard activation.','<span>Automatic</span>')}${field('Recording mode','',frameRateControl())}${field('Quality preset','Next game launch',selectControl('quality','Quality preset',['Efficient','Balanced','High'],draft.quality))}${field('File format','Next saved clip',selectControl('format','File format',['MKV','MP4'],draft.format))}${field('Initial save duration <span class="auto-saved-label">Auto-saved</span>','Selected when the replay save control opens','<select aria-label="Replay menu initial duration" data-replay-preference="initial-duration">'+durations.map(d=>`<option value="${d}" ${d===Number(replayPreferences?.initial_save_duration_seconds||30)?'selected':''}>${d<60?d+' seconds':d/60+(d===60?' minute':' minutes')}</option>`).join('')+'</select>')}${field('Microphone <span class="auto-saved-label">Auto-saved</span>','Mix your microphone input with system audio in saved clips. Applies to the next game launch; end the current session before changing this setting.',switchControl('microphone','Capture microphone',replayPreferences?.microphone_mode!=='off','replay-preference',!native||!replayPreferences||busy||Boolean(activeSession?.launch_locked)||active()))}${microphoneSelector(microphoneDevices,replayPreferences,Boolean(activeSession?.launch_locked)||Boolean(active()),errors.microphoneDevices)}${field('Replay folder <span class="auto-saved-label">Auto-saved</span>','Future clips are saved below this directory',`<span class="path-value">${escape(folder)}</span>`)}<div class="replay-pref-actions"><button class="button" data-action="change-replay-folder">Change folder</button>${button('Use default folder','reset-replay-folder',false,replayPreferences?.custom_save_parent?'':'disabled')}</div>${field('Dismiss menu on outside click <span class="auto-saved-label">Auto-saved</span>','Close the in-game Replay menu when its outside area is clicked',switchControl('outside','Dismiss menu on outside click',replayPreferences?.close_overlay_on_outside_click===true,'replay-preference',!native||!replayPreferences||busy))}</section>`;
}
function frameRateControl() {
 const selected=Number(draft.fps);
 const legacy=selected===30?[[30,'30 FPS']]:selected===120?[[120,'120 FPS']]:[];
 const options=[...legacy,[60,'60 FPS'],[240,'Variable FPS']].map(([value,label])=>`<option value="${value}" ${value===selected?'selected':''}>${label}</option>`).join('');
 return `<div class="frame-rate-control"><select aria-label="Recording mode" data-global="fps">${options}</select></div>`;
}
function globalShortcuts() {
 const labels=['Open replay menu',...durations.map(d=>`Save last ${d<60?`${d} seconds`:`${d/60} minute${d===60?'':'s'}`}`)];
 return `<section class="panel shortcuts-panel"><div class="section-heading"><div><h2>Replay shortcuts</h2><p>Global shortcuts stay active while Redunar is running, including when the window is hidden.</p></div><div class="heading-actions">${shortcutBadge()}${button('Retry activation','retry-shortcuts',false,'hidden')}${button('Clear all shortcuts','clear-shortcuts')}</div></div><div class="shortcut-rows">${labels.map((label,i)=>field(label,'',`<input class="shortcut-input" aria-label="${label}" data-shortcut="${i}" value="${escape(draftShortcuts[i])}" spellcheck="false">`)).join('')}</div><p class="small-note" id="shortcut-feedback">${escape(shortcutStatus?.message||shortcutStatus?.last_action||'Shortcut assignments are stored locally and monitored by Redunar while the app is running.')}</p></section>`;
}
function historyPeer(selected) {
 return sessions.find((candidate,index)=>index!==selectedSession&&candidate.game===selected.game&&String(candidate.id)===historyPeerId) || null;
}
function momentValue(value,digits,unit='') {
 return Number.isFinite(value)?`${value.toFixed(digits)}${unit}`:'—';
}
function historySampleStatus(sample,legacy) {
 if(!sample)return 'No observation recorded at this position.';
 return legacy?'Reconstructed from retained frame intervals.':'Recorded observation · FPS is a window average; frame time is the latest captured frame.';
}
function loadValue(value) { return Number.isFinite(value)?`${Number(value.toFixed(4))}%`:'—'; }
function historyMoment(sample,legacy=false,cursor=0) {
 return `<div class="history-moment"><div class="history-moment-heading"><span>Selected moment</span><strong id="history-cursor-time">${historyTime(cursor)} into session</strong></div><div class="history-moment-grid"><div><span>FPS</span><b id="history-moment-fps">${momentValue(sample?.fps,1)}</b></div><div><span>Frame time</span><b id="history-moment-frame">${momentValue(sample?.frameTime,1,' ms')}</b></div><div><span>CPU temperature</span><b id="history-moment-cpu-temp">${momentValue(sample?.cpuTemperature,1,'°C')}</b></div><div><span>GPU temperature</span><b id="history-moment-gpu-temp">${momentValue(sample?.gpuTemperature,1,'°C')}</b></div><div><span>CPU load</span><b id="history-moment-cpu-load">${loadValue(sample?.cpuUtilization)}</b></div><div><span>GPU load</span><b id="history-moment-gpu-load">${loadValue(sample?.gpuUtilization)}</b></div></div><p id="history-sample-status">${historySampleStatus(sample,legacy)}</p>${legacy?'<p>Older session · FPS is reconstructed from retained frame intervals. Temperature and load samples were not recorded.</p>':''}</div>`;
}
function aggregateTimelinePoints(points,duration) {
 if(points.length<3)return points;
 const bucketCount=Math.min(96,Math.max(24,Math.round(Math.sqrt(points.length)*2)));
 const buckets=Array.from({length:bucketCount},()=>[]);
 points.forEach(point=>{
  const bucket=Math.min(bucketCount-1,Math.max(0,Math.floor(point.x/Math.max(1,duration)*bucketCount)));
  buckets[bucket].push(point);
 });
 return buckets.filter(bucket=>bucket.length).map(bucket=>{
  const values=bucket.map(point=>point.value).sort((a,b)=>a-b);
  const middle=Math.floor(values.length/2);
  const value=values.length%2?values[middle]:(values[middle-1]+values[middle])/2;
  return {x:bucket.reduce((sum,point)=>sum+point.x,0)/bucket.length,value};
 });
}
function historyScrubber(samples,cursor,duration) {
 return `<div class="history-scrubber-wrap"><div class="history-scrubber-heading"><label for="history-scrubber">Selected time <output id="history-slider-time">${historyTime(cursor)}</output></label><span>Drag or use arrow keys to inspect observations</span></div><input id="history-scrubber" class="history-scrubber" type="range" data-history-cursor min="0" max="${duration}" step="any" value="${cursor}" aria-valuetext="${historyTime(cursor)} into session" ${!samples.length||!duration?'disabled':''}></div>`;
}
function historyTimelineChart(samples,metric,cursor,duration) {
 const definition=historyMetrics[metric]||historyMetrics.fps;
 const lines=definition.series.map(([key,label,kind])=>({label,kind,segments:historySeries(samples,key)}));
 const scale=historyScale(lines.flatMap(line=>line.segments.flatMap(segment=>segment.map(point=>point.value))),metric);
 const scrubber=historyScrubber(samples,cursor,duration);
 if(!scale)return `<div class="chart empty-chart"><p>${samples.length?'This metric was unavailable during the session.':'This session has no timeline samples.'}</p></div>${scrubber}`;
 const traces=lines.map(line=>line.segments.map(segment=>{
  const points=segment.map(point=>`${historyX(point.x,duration)},${historyY(point.value,scale)}`).join(' ');
  const first=historyX(segment[0].x,duration),last=historyX(segment.at(-1).x,duration);
  if(segment.length===1)return `<circle cx="${first}" cy="${historyY(segment[0].value,scale)}" r="3" class="timeline-trace timeline-observation ${line.kind}"/>`;
  const area=line.kind==='primary'?`<polygon points="${first},210 ${points} ${last},210" class="timeline-area"/>`:'';
  return `${area}<polyline points="${points}" class="timeline-trace ${line.kind}" aria-label="${line.label}"/>`;
 }).join('')).join('');
 const marker=historyX(cursor,duration);
 return `<div class="history-chart-legend">${lines.map(line=>`<span class="${line.kind}"><i></i>${line.label}</span>`).join('')}</div><div class="chart history-timeline-chart"><div class="y-axis">${scale.ticks.map(value=>`<span data-axis-value="${value}" style="top:${historyY(value,scale)/220*100}%">${value.toFixed(scale.digits)} ${scale.unit}</span>`).join('')}</div><svg viewBox="0 0 900 220" preserveAspectRatio="none" role="img" aria-label="Session ${escape(metric)} timeline"><defs><linearGradient id="history-area-gradient" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f15c6c" stop-opacity=".2"/><stop offset="1" stop-color="#f15c6c" stop-opacity="0"/></linearGradient></defs>${scale.ticks.map(value=>`<path d="M0 ${historyY(value,scale)}H900" class="gridline" data-axis-value="${value}"/>`).join('')}${traces}<line id="history-cursor-line" x1="${marker}" x2="${marker}" y1="12" y2="214" class="history-cursor-line"/></svg></div><div class="chart-axis history-time-axis"><span>${historyTime(0)}</span><span>${historyTime(duration/2)}</span><span>${historyTime(duration)}</span></div>${scrubber}`;
}
function updateHistoryCursor(value) {
 const session=sessions[selectedSession];if(!session)return;
 const timeline=historyTimeline(session),duration=historyDuration(session,timeline.samples);
 const selection=historySelection(timeline.samples,value,duration),sample=selection.sample;
 historyCursor=selection.seconds;
 const marker=$('#history-cursor-line');if(marker){const x=historyX(historyCursor,duration);marker.setAttribute('x1',x);marker.setAttribute('x2',x);}
 const slider=$('[data-history-cursor]');if(slider){slider.value=historyCursor;slider.setAttribute('aria-valuetext',`${historyTime(historyCursor)} into session`);updatePrecisionSlider(slider);}
 const values={
  '#history-cursor-time':`${historyTime(historyCursor)} into session`,
  '#history-slider-time':historyTime(historyCursor),
  '#history-sample-status':historySampleStatus(sample,timeline.legacy),
  '#history-moment-fps':momentValue(sample?.fps,1),
  '#history-moment-frame':momentValue(sample?.frameTime,1,' ms'),
  '#history-moment-cpu-temp':momentValue(sample?.cpuTemperature,1,'°C'),
  '#history-moment-gpu-temp':momentValue(sample?.gpuTemperature,1,'°C'),
  '#history-moment-cpu-load':loadValue(sample?.cpuUtilization),
  '#history-moment-gpu-load':loadValue(sample?.gpuUtilization)
 };
 for(const [selector,text] of Object.entries(values)){const element=$(selector);if(element)element.textContent=text;}
}
// Native range steps are uniform; journal timestamps are not (especially after
// retention compaction). Move by observations so keyboard users cannot get stuck.
document.addEventListener('keydown',event=>{
 if(!event.target.matches('[data-history-cursor]')||event.target.disabled)return;
 const session=sessions[selectedSession];if(!session)return;
 const {samples}=historyTimeline(session),duration=historyDuration(session,samples);
 const value=historyKeyboardTime(samples,Number(event.target.value),event.key,duration);
 if(value===null)return;
 event.preventDefault();updateHistoryCursor(value);
});
function average(values) {
 return values.length ? values.reduce((sum,value)=>sum+value,0)/values.length : null;
}
function deltaText(current,previous,unit='') {
 if(!Number.isFinite(current)||!Number.isFinite(previous))return '—';
 const delta=current-previous;
 return `${delta>=0?'+':''}${delta.toFixed(1)}${unit}`;
}
function historyComparison(selected,peer) {
 if(!historyCompare||!peer)return '';
 const selectedFrame=average(selected.frames),peerFrame=average(peer.frames);
 return `<div class="history-comparison"><div class="section-heading"><div><h3>Compared with another ${escape(selected.game)} session</h3><p>${escape(peer.date)} at ${escape(peer.time)}</p></div>${pill('Observed difference')}</div><div class="comparison-grid"><div><span>Average FPS</span><b>${deltaText(selected.averageFps,peer.averageFps,' FPS')}</b></div><div><span>1% low</span><b>${deltaText(selected.onePercentLowFps,peer.onePercentLowFps,' FPS')}</b></div><div><span>0.1% low</span><b>${deltaText(selected.pointOnePercentLowFps,peer.pointOnePercentLowFps,' FPS')}</b></div><div><span>Retained frame time</span><b>${deltaText(selectedFrame,peerFrame,' ms')}</b></div></div><p class="small-note">Frame time compares only the retained frame intervals. FPS summaries use the recorded session summary. They do not establish a cause or a guaranteed performance change.</p></div>`;
}
function historyCatalog() { return `<aside class="history-catalog"><label class="search">${icon('search')}<input id="history-search" type="search" placeholder="Find a session" aria-label="Find a session" value="${escape(historyFilters.search)}"></label>${historyFiltersMarkup(sessions,historyFilters)}<div class="catalog-label" id="history-result-count">${filterSessions(sessions,historyFilters).length} of ${sessions.length} sessions</div><div id="session-list">${historyRows()}</div><div class="catalog-note">${icon('history')}<p>Your history stays on this device.</p></div></aside>`; }
function history() {
 if(sessions.length&&!filterSessions(sessions,historyFilters).length)return heading('History','Review session measurements and compare recorded runs.',button('Refresh history','reload-history'))+`<div class="history-workspace">${historyCatalog()}</div>`;
 const s=sessions[selectedSession];
 if(!s)return heading('History','Review session measurements and compare recorded runs.',button('Refresh history','reload-history'))+emptyPanel(errors.history?'History unavailable':'No recorded sessions yet.',errors.history||'Launch a game through Redunar or configured Steam. Its measurements will appear here when the session ends.');
 const peer=historyPeer(s),timeline=historyTimeline(s),duration=historyDuration(s,timeline.samples);
 if(historyCursor===null||historyCursor>duration)historyCursor=timeline.samples.at(-1)?.elapsed??0;
 const selection=historySelection(timeline.samples,historyCursor,duration),moment=selection.sample;
 historyCursor=selection.seconds;
 return heading('History','Review session measurements and compare recorded runs.',button('Refresh history','reload-history'))+
 `<div class="history-workspace">${historyCatalog()}<section class="panel history-detail"><div class="section-heading"><div><p class="record-label">Recorded session</p><h2>${escape(s.game)}</h2><p>${s.date} at ${s.time}</p></div>${pill('Completed','green')}</div><div class="telemetry-row history-stats">${stat('Duration',s.duration)}${stat('Average',s.fps,'FPS')}${stat('1% low',s.low,'FPS')}${stat('0.1% low',s.lowest,'FPS')}</div><div class="history-controls"><div class="history-view-toggle" role="group" aria-label="History chart metric">${[['fps','FPS'],['frame-time','Frame time'],['temperature','Temperatures'],['utilization','Load']].map(([value,label])=>`<button class="${historyMetric===value?'active':''}" data-history-view="${value}" aria-pressed="${historyMetric===value}">${label}</button>`).join('')}</div>${comparisonSelector(sessions,selectedSession,historyPeerId)}</div><div class="section-heading"><div><h3>Session timeline</h3><p>${timeline.legacy?'Legacy retained frame sequence':'Sampled throughout the complete session'}</p></div><span>${timeline.samples.length} observations</span></div>${historyTimelineChart(timeline.samples,historyMetric,historyCursor,duration)}${historyMoment(moment,timeline.legacy,historyCursor)}${historyComparison(s,peer)}<div class="history-explanation"><span>${icon('history')}</span><div><h3>Move through the complete session.</h3><p>Drag the timeline to inspect frame rate, frame time, temperatures, and hardware load at that moment. Long sessions retain the full time span with gradually reduced sample resolution.</p></div></div></section></div>`;
}
function historyRows() {
 return filterSessions(sessions,historyFilters).map(s=>`<button class="history-row ${s.index===selectedSession?'selected':''}" data-session="${s.index}" aria-pressed="${s.index===selectedSession}"><small>${s.date}</small><strong>${escape(s.game)}</strong><span>${s.duration}<b>${s.fps} FPS</b></span></button>`).join('') || '<div class="history-filter-empty"><strong>No matching sessions</strong><button class="button" data-action="clear-history-filters">Clear filters</button></div>';
}
function refreshHistoryRail() {
 const visible=filterSessions(sessions,historyFilters);
 if(!visible.some(s=>s.index===selectedSession)||!$('.history-detail')){
  const input=$('#history-search'),focused=document.activeElement===input,caret=input?.selectionStart;
  if(visible.length){selectedSession=visible[0].index;historyCursor=null;historyPeerId='';historyCompare=false;}
  render();if(focused){const search=$('#history-search');search?.focus();if(search&&caret!==null)search.setSelectionRange(caret,caret);}return true;
 }
 const list=$('#session-list');if(list)list.innerHTML=historyRows();
 const count=$('#history-result-count');if(count)count.textContent=`${filterSessions(sessions,historyFilters).length} of ${sessions.length} sessions`;
}

function updateCardMarkup() {
 return updateCard(updateStatus,{native,busy,restartBlocked:Boolean(active()||activeSession?.launch_locked),pending:pendingUpdateAction});
}
function refreshUpdateCard() {
 runningVersion=updateStatus?.current_version||runningVersion;
 const sidebar=$('.build-label');
 const sidebarMarkup=updateSidebar(updateStatus,{version:runningVersion,pending:pendingUpdateAction});
 if(sidebar.updateMarkup!==sidebarMarkup){sidebar.innerHTML=sidebarMarkup;sidebar.updateMarkup=sidebarMarkup;const badge=$('.sidebar-update-badge',sidebar);const label=`Settings · Version ${runningVersion}${badge?' · '+badge.getAttribute('aria-label'):''}`;$('.settings-link').setAttribute('aria-label',label);$('.settings-link').title=label;}
 const card=$('[data-update-card]');if(!card)return;
 const markup=updateCardMarkup();
 // Keep controls and focus intact when polling has not changed the visible state.
 if(card.updateMarkup!==markup){card.innerHTML=markup;card.updateMarkup=markup;}
}
async function executeUpdate(command,args={}) {
 try {
  const result=await call(command,args);
  if(command==='restart_redunar'){notify('Restarting Redunar…');return;}
  else updateStatus={...updateStatus,...result,operation_issue:command==='install_update'&&result.state!=='restart-needed'};
  notify(updateStatus.message);
 } catch(error) {
  updateStatus={...updateStatus,state:'error',message:message(error)};
  reportActionError(error);
 } finally {if(command!=='restart_redunar'||updateStatus?.state==='error')pendingUpdateAction=null;}
}
async function performUpdate(command,pending,args={}) {
 if(busy||pendingUpdateAction)return;
 pendingUpdateAction=pending;
 await perform(()=>executeUpdate(command,args));
}
async function saveGlobalProfileDraft() {
 if(!globalProfileChanged())return;
 const pendingShortcuts=[...draftShortcuts],previousShortcuts=[...shortcuts],previousMap={...savedShortcutMap};
 const saved=await call('save_global_settings',{input:profilePayload(draft,savedShortcutMap),expectedRevision:globalRevision});
 acceptGlobal(saved);
 // A profile save must not silently accept an independent shortcut draft.
 draftShortcuts=pendingShortcuts;shortcuts=previousShortcuts;savedShortcutMap=previousMap;
 try{storageStatus=await call('replay_storage_status');delete errors.storage;}catch(error){storageStatus=null;errors.storage=message(error);}
 return saved;
}
function shortcutDraftPayload() {
 const next=Object.fromEntries([['overlay',draftShortcuts[0]],...durations.map((d,i)=>[String(d),draftShortcuts[i+1]])].filter(([,value])=>value.trim()));
 const normalized=Object.values(next).map(v=>v.replaceAll(' ','').toLowerCase());
 if(new Set(normalized).size!==normalized.length)throw new Error('Use a different shortcut for each assigned action.');
 return next;
}
async function saveShortcutDraft() {
 if(!globalShortcutsChanged())return;
 const next=shortcutDraftPayload();
 const result=await call('save_shortcuts',{shortcuts:next,expected:savedShortcutMap});
 savedShortcutMap={...result.values.shortcuts};shortcuts=[savedShortcutMap.overlay||'',...durations.map(d=>savedShortcutMap[d]||'')];draftShortcuts=[...shortcuts];
 try{shortcutStatus=await call('activate_shortcuts');return Object.keys(next).length?'Shortcut assignments saved and activated.':'All shortcuts cleared. Replay remains available from the app.';}
 catch(error){shortcutStatus=await call('shortcut_status').catch(()=>shortcutStatus);return `Shortcut assignments saved, but activation failed: ${message(error)}`;}
}
async function saveGameProfileDraft(gameId) {
 const value=games.find(g=>g.id===gameId);if(!gameProfileChanged(value))return;
 const saved=await call('save_game_profile',{gameId,values:overridePayload(value.overrides),expectedRevision:value.revision});
 acceptGames(saved.games,gameId);
 return saved;
}
const hasRestartDrafts=()=>globalProfileChanged()||globalShortcutsChanged()||games.some(gameProfileChanged);
function requestUpdateRestart() {
 if(busy||active()||activeSession?.launch_locked||updateStatus?.state!=='restart-needed')return;
 if(!hasRestartDrafts()){performUpdate('restart_redunar','restarting');return;}
 modal('Unsaved changes',`<p>Save your settings before restarting?</p><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button" type="button" data-action="discard-and-restart">Discard and restart</button><button class="button primary" type="button" data-action="save-and-restart">Save and restart</button></div>`);
}
async function finishDraftRestart(save) {
 if(busy||active()||activeSession?.launch_locked||updateStatus?.state!=='restart-needed')return;
 await perform(async()=>{
  if(save){
   // Validate all shortcut assignments before making any persistent writes.
   if(globalShortcutsChanged())shortcutDraftPayload();
   await saveGlobalProfileDraft();await saveShortcutDraft();
   for(const id of games.filter(gameProfileChanged).map(g=>g.id))await saveGameProfileDraft(id);
  }
  // A session may have started while these asynchronous saves were pending.
  if(active()||activeSession?.launch_locked)throw new Error('End the session to restart.');
  if(!save){
   draft=structuredClone(defaults);draftShortcuts=[...shortcuts];
   for(const g of games){g.overrides={...savedGameOverrides.get(g.id)};g.revision=g.savedRevision??g.revision;}
  }
  $('#dialog').close();pendingUpdateAction='restarting';updateObservedElements();
  await executeUpdate('restart_redunar');
 });
}
function settings() {
 const preferenceRow=(title,description,note,key,checked,extra='')=>`<div class="settings-option"><div class="settings-option-main"><div class="settings-option-copy"><h3>${title}</h3><p>${description}</p></div>${switchControl(key,title,checked,'preference',!native||!loaded.preferences||busy)}</div><p class="settings-option-note">${note}</p>${extra}</div>`;
 return heading('Settings','Desktop preferences and software updates.')+
`<div class="settings-grid settings-page">
 <section class="panel settings-preferences"><div class="settings-card-heading"><h2>Preferences</h2><p>Choose how Redunar runs on this device.</p></div>
  ${preferenceRow('Close to tray','Keep Redunar running when you close the window.','Use the tray menu to reopen or quit Redunar. When this is off, Steam startup and closing the window during a game use a temporary tray icon. Redunar quits when the game ends unless you reopen it.','tray',preferences.tray===true)}
  ${preferenceRow('Debug log','Record operational messages for support.',`Logs can include game names and session details. <span id="diagnostic-log-status">${escape(diagnosticDisplayStatus())}</span>`,'diagnostic-log',preferences.diagnosticLog===true,preferences.diagnosticLogPath?`<button class="button settings-log-action" type="button" data-action="open-diagnostic-log-folder">Open log folder</button>`:'')}
 </section>
 <section id="software-updates" class="panel controls-panel updates-panel" tabindex="-1" aria-labelledby="software-updates-heading"><div class="updates-heading"><h2 id="software-updates-heading">Software updates</h2></div><div data-update-card>${updateCardMarkup()}</div><div class="update-preferences"><div class="feature-access-row"><label for="feature-access">Feature access</label><select id="feature-access" aria-label="Feature access" ${!native||!loaded.preferences||busy?'disabled':''}><option value="standard" ${preferences.betaAccess?'':'selected'}>Stable</option><option value="beta" ${preferences.betaAccess?'selected':''}>Beta</option></select></div>${field('Check on startup','',switchControl('automatic-updates','Check for updates automatically',preferences.automaticUpdates!==false,'preference',!native||!loaded.preferences||busy))}</div></section>
</div>`;
}

function diagnosticsCard() {
 if(!native) return `<section class="panel diagnostics-panel"><h2>Diagnostics</h2><p>Open the native app to inspect monitor, capture, and encoder readiness.</p></section>`;
 if(!diagnostics) return `<section class="panel diagnostics-panel"><h2>Diagnostics</h2><p>Collecting read-only runtime counters…</p></section>`;
 const readiness=diagnostics.replay_readiness||{};
 const readyCount=Object.values(readiness).filter(Boolean).length;
 const candidateText=diagnostics.encoder_candidates?.length?diagnostics.encoder_candidates.map(c=>`${escape(c.driver)}${c.accessible?' · accessible':' · unavailable'}`).join(', '):'No eligible render node candidates found';
 const issue=diagnostics.telemetry_error||diagnostics.game_detection_error;
 const readinessRows=[['encoded_packet_ring','Encoded packet ring'],['atomic_clip_store','Atomic clip store'],['live_frame_transfer','Live frame transfer'],['hardware_encoder','Hardware encoder'],['playable_container','Playable container'],['performance_budget','Performance budget']];
 const captured=diagnostics.captured_at_unix_ms==null?'—':new Date(diagnostics.captured_at_unix_ms).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit',second:'2-digit'});
 const status=value=>pill(value?'Ready':'Not verified',value?'green':'red');
 const shortcutNote=diagnostics.shortcut_message||'Monitored by Redunar while the app is running; no desktop shortcut registry is used.';
 return `<section class="panel diagnostics-panel"><div class="section-heading"><div><h2>Diagnostics</h2><p>Read-only counters from the native workers.</p></div>${button('Refresh','reload-diagnostics')}</div><div class="diagnostics-grid"><div><span>Monitor samples</span><b>${diagnostics.hardware_samples}</b></div><div><span>Game scans</span><b>${diagnostics.game_scans}</b></div><div><span>Replay readiness</span><b>${readyCount}/6 ready</b></div><div><span>Capture runtime</span><b>${escape(diagnostics.capture_runtime)}</b></div><div><span>Shortcut monitor</span><b>${escape(diagnostics.shortcut_state||'Unknown')}</b></div><div><span>Frames received</span><b>${diagnostics.received_frame_count}</b></div><div><span>Encoded packets</span><b>${diagnostics.encoded_packet_count}</b></div><div><span>Audio packets</span><b>${diagnostics.audio_packet_count}</b></div><div><span>Audio bytes</span><b>${diagnostics.audio_byte_count}</b></div></div><div class="diagnostics-readiness"><div class="diagnostics-subheading"><h3>Replay readiness checks</h3><span>Backend verification</span></div>${readinessRows.map(([key,label])=>`<div class="diagnostics-readiness-row"><span>${label}</span>${status(Boolean(readiness[key]))}</div>`).join('')}</div><p class="small-note">Last snapshot ${captured} · sample ${diagnostics.last_hardware_sample_ms==null?'—':diagnostics.last_hardware_sample_ms+' ms'} · scan ${diagnostics.last_game_scan_ms==null?'—':diagnostics.last_game_scan_ms+' ms'} · overruns ${diagnostics.hardware_overruns+diagnostics.game_scan_overruns}</p><p class="small-note">Shortcut monitor: ${escape(shortcutNote)}</p><p class="small-note">Encoder candidates: ${candidateText}</p>${issue?`<p class="diagnostics-error">${escape(issue)}</p>`:''}</section>`;
}

function render() {
 const samePage=document.body.dataset.page===route();
 const filters=samePage?['game-search','clip-search'].map(id=>[id,document.getElementById(id)?.value]).filter(([,value])=>value):[];
 const rails=samePage?['#library-list','#session-list','.clip-list'].map(selector=>[selector,$(selector)?.scrollTop??0]):[];
 closePrecisionSelect();
 resetPlayback();
 const current=route();
 const navigationPages=visiblePages();
 const valid=[...navigationPages.map(p=>p[0]),'settings'].includes(current)?current:'overview';
 if(current==='replay'&&valid==='overview')history.replaceState(null,'','#overview');
 $('#navigation').innerHTML=navigationPages.map(([id,label])=>`<a href="#${id}" aria-label="${label}" title="${label}" class="nav-link ${valid===id?'active':''}" ${valid===id?'aria-current="page"':''}>${icon(id)}<span>${label}</span>${id==='global'?'<i class="nav-unsaved-marker" aria-hidden="true" hidden></i>':''}${id==='replay'?`<span class="nav-count">${clips.length}</span>`:''}</a>`).join('');
 $('.settings-link').classList.toggle('active',valid==='settings');
 workspace.innerHTML=({overview,library,global:globalSettings,replay,history,settings}[valid])();
 document.title=`Redunar — ${pages.find(p=>p[0]===valid)?.[1] || 'Settings'}`;
 document.body.dataset.page=valid;
 document.documentElement.dataset.page=valid;
 if(valid==='replay'){loadVideo();scheduleClipThumbnails();}
 if(valid==='overview')scheduleClipThumbnails();
 if(valid==='global')updateHud();
 updateObservedElements();
 const ready=valid==='global'?loaded.global:valid==='library'?loaded.catalog:true;
 if(!native||!ready)workspace.querySelectorAll('button[data-action],input,select').forEach(el=>{if(!native||!el.dataset.action?.startsWith('reload'))el.disabled=true;});
 for(const [id,value] of filters){const input=document.getElementById(id);if(input){input.value=value;input.dispatchEvent(new Event('input',{bubbles:true}));}}
 for(const [selector,top] of rails){const rail=$(selector);if(rail)rail.scrollTop=top;}
 enhancePrecisionSelects(workspace);updatePrecisionSliders(workspace);
 if(valid==='settings'&&focusUpdatesOnRender){focusUpdatesOnRender=false;$('#software-updates').scrollIntoView({block:'nearest'});$('#software-updates').focus({preventScroll:true});}
 if(valid==='library'){loadGameArtwork(workspace,call);if(native&&loaded.catalog){installation.refresh();refreshSteamSetup($('.steam-setup-card'),game()?.id,call);}}
 if(valid==='global'&&!loaded.global)workspace.insertAdjacentHTML('afterbegin',`<p class="connection-warning">${escape(errors.global||'Loading your saved defaults…')}</p>`);
}
function notify(text) {
 $('#toast').textContent=text;$('#toast').hidden=false;
 clearTimeout(toastTimer);toastTimer=setTimeout(()=>$('#toast').hidden=true,4200);
}
function modal(title,body) {
 $('#dialog-error').hidden=true;$('#dialog-error').textContent='';
 $('#dialog-title').textContent=title;$('#dialog-body').innerHTML=body;
 $('#dialog').setAttribute('aria-labelledby','dialog-title');$('#dialog-title').setAttribute('tabindex','-1');$('#dialog-title').setAttribute('autofocus','');$('#dialog').showModal();enhancePrecisionSelects($('#dialog'));$('#dialog-title').focus({preventScroll:true});
}
function discoveryModal(candidates) {
 // Steam games share a launcher executable; only their app ID identifies the game.
 const alreadyImported=candidate=>games.some(game=>candidate.source==='Steam'
  ? candidate.source_id!=null&&game.steam_app_id===candidate.source_id
  : Boolean(candidate.launch_executable&&game.executable===candidate.launch_executable));
  const rows=candidates.map(candidate=>{const existing=alreadyImported(candidate),importable=candidate.importable&&!existing;return `<label class="discovery-row ${importable?'':'unavailable'}"><input type="checkbox" name="candidateId" value="${escape(candidate.candidate_id)}" ${importable?'checked':''} ${importable?'':'disabled'}><span><strong>${escape(candidate.name)}</strong><small>${escape(candidate.source)}${candidate.source_id?` · ${candidate.source_id}`:''} · ${escape(candidate.install_directory)}</small>${existing?'<em>Already in Redunar library</em>':candidate.importable?`<code>${escape(candidate.launch_executable||'')}</code>`:'<em>No verified launch found on this system</em>'}</span></label>`}).join('');
 const hasImportable=candidates.some(c=>c.importable&&!alreadyImported(c));
 modal('Scan installed games',`<p>Review installed games found in local Steam libraries and desktop entries before adding them to your Redunar library. Discovery is read-only until you import the selected games.</p><form id="discovery-form"><h3>Installed entries</h3><div class="discovery-list">${rows||'<p class="empty">No installed games were found.</p>'}</div><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit" ${hasImportable?'':'disabled'}>Import selected</button></div></form>`);
}
function stopPlayback(){ const video=$('#clip-video');if(video)video.pause(); }
function resetPlayback(){
 stopFilmstrip?.();stopFilmstrip=null;
 playbackRequests.invalidate();
 mediaEvents?.abort();mediaEvents=null;
 mediaActionGeneration++;seekGeneration++;
 if(cancelPendingSeek)cancelPendingSeek();
 cancelPendingSeek=null;pendingSeek=null;pendingSeekTask=Promise.resolve();
 stopPlayback();stopPlaybackTicker();
 const video=$('#clip-video');
 if(video){video.removeAttribute('src');video.load();}
 videoReady=false;playRequested=false;playerDuration=0;playerPosition=0;trimStart=0;trimEnd=0;timelineDrag=null;
}
function selectClip(id){
 if(id===selectedClip||!clips.some(item=>item.id===id))return;
 selectedClip=id;
 // Leave the rail, focused card, filter and save-duration control mounted.
 reloadSelectedClip();
 document.querySelectorAll('[data-clip]').forEach(button=>{
  const selected=button.dataset.clip===id;
  button.setAttribute('aria-pressed',String(selected));
  button.closest('.clip-card').classList.toggle('selected',selected);
 });
 scheduleClipThumbnails();
}
function reloadSelectedClip(){
 const viewport=$('.window-content'),top=viewport?.scrollTop??0;
 resetPlayback();
 $('#replay-selection').innerHTML=replaySelection(clip());
 loadVideo();
 // Replacing a loaded player with its pending state changes its height.
 // Preserve the workspace position instead of following browser anchoring.
 if(viewport)viewport.scrollTop=top;
}
function scheduleClipThumbnails(){
 clearTimeout(thumbnailLoadTimer);
 thumbnailLoadTimer=setTimeout(()=>{thumbnailLoadTimer=null;if(['overview','replay'].includes(route()))loadClipThumbnails();},800);
}
function updateHud() {
 const previewScale=$('#overlay-preview-scale');if(previewScale)previewScale.textContent=draft.scale;
 const validation=$('#metric-validation');if(validation)validation.textContent=draft.preset==='Custom'&&!draft.metrics.length?'Select at least one metric to save a Custom layout.':'';
 const hint=$('.metric-hint');if(hint)hint.textContent=draft.preset==='Custom'?'Choose the measurements to display.':'Choose Custom to change individual metrics.';
 const hud=$('#hud');
 if(hud){
  const palette=overlayPalettes[draft.palette]||overlayPalettes.Redunar;
  hud.innerHTML=hudContent();hud.hidden=!draft.overlay;hud.dataset.position=draft.position;hud.dataset.preset=draft.preset||'Compact';hud.dataset.layout=draft.layout||'Grid';
  hud.style.setProperty('--hud-native-height',`${overlayHeight()}px`);hud.style.setProperty('--hud-scale',draft.scale/100);hud.style.setProperty('--hud-preview-fit',draft.layout==='Ribbon'?.78:1);hud.style.setProperty('--hud-ribbon-width',`${ribbonWidth()}px`);hud.style.setProperty('--hud-opacity',draft.opacity/100);
  [['--hud-accent',palette[0]],['--hud-panel',palette[1]],['--hud-text',palette[2]],['--hud-muted',palette[3]],['--hud-border',palette[4]]].forEach(([key,value])=>hud.style.setProperty(key,value));
  document.querySelectorAll('[data-layout]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.layout===draft.layout)));
  document.querySelectorAll('[data-palette]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.palette===draft.palette)));
  if($('#overlay-preview-layout'))$('#overlay-preview-layout').textContent=draft.layout||'Grid';
  if($('#overlay-preview-palette'))$('#overlay-preview-palette').textContent=draft.palette||'Redunar';
  if($('#overlay-palette-name'))$('#overlay-palette-name').textContent=draft.palette||'Redunar';
  for(const key of ['scale','opacity'])if($(`#${key}-value`))$(`#${key}-value`).textContent=`${draft[key]}%`;
 }
 updateDirtyActionButtons();
}
function syncMetricToggles() {
 document.querySelectorAll('[data-preset]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.preset===draft.preset)));
 const custom=draft.preset==='Custom';
 const shown=new Set(effectiveMetrics(draft.preset||'Compact',draft.metrics));
 document.querySelectorAll('[data-metric]').forEach(input=>{input.checked=shown.has(input.dataset.metric);input.disabled=!custom;});
}

function acceptGlobal(result) {
 defaults=mapDefaults(result); draft=structuredClone(defaults); globalRevision=result.revision;
 savedShortcutMap={...result.values.shortcuts};
 shortcuts=[savedShortcutMap.overlay||'',...durations.map(d=>savedShortcutMap[d]||'')];
 draftShortcuts=[...shortcuts]; loaded.global=true;
}
function acceptGames(records,committedId=null) {
 const saved=mapGames(records);
 games=mergeGameDrafts(saved,games,savedGameOverrides,committedId);loaded.catalog=true;
 savedGameOverrides=new Map(saved.map(item=>[item.id,{...item.overrides}]));
 if(!game())selectedGame=games[0]?.id??null;
}
function acceptClips(records) {
 const previous=new Map(clips.map(item=>[item.id,item]));
 clips=mapClips(records).map(item=>{
  const cached=previous.get(item.id);
  if(cached&&cached.bytes===item.bytes&&cached.modified_unix_ns===item.modified_unix_ns){item.thumbnail=cached.thumbnail;item.duration=cached.duration;item.metadata=cached.metadata;item.metadataDetailed=cached.metadataDetailed;}
  return item;
 });loaded.clips=true;
 if(!clip())selectedClip=clips[0]?.id??null;
}
function thumbnailDataUri(bytes) {
 const values=bytes instanceof Uint8Array?bytes:Array.isArray(bytes)?Uint8Array.from(bytes):null;
 if(!values||!values.length)return null;
 let binary='';
 for(let offset=0;offset<values.length;offset+=0x8000)binary+=String.fromCharCode(...values.subarray(offset,offset+0x8000));
 return `data:image/jpeg;base64,${btoa(binary)}`;
}
async function loadClipThumbnails() {
 if(!native)return;
 if(thumbnailLoadInFlight){thumbnailReloadRequested=true;return;}
 thumbnailLoadInFlight=true;thumbnailReloadRequested=false;
 try{
  // Keep the replay page responsive while FFmpeg creates previews. The rail
  // only shows a few cards at once, so prioritize those and the selected clip.
  const rail=$('.clip-list'),bounds=rail?.getBoundingClientRect();
  const visible=Array.from(document.querySelectorAll('[data-clip]')).filter(card=>{const rect=card.getBoundingClientRect();return bounds&&rect.bottom>=bounds.top&&rect.top<=bounds.bottom;}).map(card=>card.dataset.clip);
  const prioritized=[selectedClip,...(route()==='overview'?clips.slice(0,3).map(item=>item.id):visible)].filter((id,index,array)=>id&&array.indexOf(id)===index);
  const pending=prioritized.map(id=>clips.find(item=>item.id===id)).filter(item=>item&&(item.thumbnail===null||(route()==='replay'&&item.id===selectedClip&&!item.metadataDetailed))).slice(0,4);
  for(let offset=0;offset<pending.length;offset+=2){
   if(!['overview','replay'].includes(route()))break;
   await Promise.all(pending.slice(offset,offset+2).map(async item=>{
    await loadClipPreview({item,call,thumbnailDataUri,updateMetadata:updateClipMetadata,updateThumbnail:updateClipThumbnail,selected:current=>clips.includes(current)&&current.id===selectedClip&&route()==='replay'});
   }));
  }
 } finally {
  thumbnailLoadInFlight=false;
  if(thumbnailReloadRequested&&['overview','replay'].includes(route()))scheduleClipThumbnails();
 }
}
function updateClipMetadata(item){
 if(!clips.includes(item))return;
 for(const card of document.querySelectorAll('[data-clip],[data-recent-clip]')){if((card.dataset.clip||card.dataset.recentClip)!==item.id)continue;const badge=card.querySelector('[data-clip-duration]');if(badge){badge.textContent=time(item.duration);badge.title=item.duration==null?'Duration unavailable':'';}}
 if(item.id===selectedClip){const specs=$('#clip-video-specs'),meta=item.metadata;if(specs&&meta?.width&&meta?.height)specs.textContent=`${meta.width} × ${meta.height}${meta.fps?' · '+measurement(meta.fps,1)+' avg recorded FPS':''}`;}
}
function updateClipThumbnail(item){
 if(item.id===selectedClip&&item.thumbnail){const video=$('#clip-video');if(video)video.poster=item.thumbnail;}
 if(route()==='overview'&&item.thumbnail){const card=Array.from(document.querySelectorAll('[data-recent-clip]')).find(card=>card.dataset.recentClip===item.id);const target=card?.querySelector('.recent-thumbnail');if(target&&!target.querySelector('img')){const image=document.createElement('img');image.src=item.thumbnail;image.alt='';image.decoding='async';target.querySelector('.icon')?.replaceWith(image);}return;}
 if(route()!=='replay'||!clips.includes(item)||!item.thumbnail)return;
 const button=Array.from(document.querySelectorAll('[data-clip]')).find(button=>button.dataset.clip===item.id);
 const thumbnail=button?.querySelector('.thumbnail');
 if(!thumbnail||thumbnail.querySelector('img'))return;
 const image=document.createElement('img');
 image.src=item.thumbnail;image.alt=`Thumbnail for ${clipTitle(item)}`;image.decoding='async';
 thumbnail.querySelector('.icon')?.replaceWith(image);
}

async function readSection(section, command, accept) {
 try { const result=await call(command); accept(result); delete errors[section]; return true; }
 catch(error) {
  errors[section]=message(error); loaded[section]=false;
  if(section==='catalog'){games=[];selectedGame=null;}
  if(section==='clips'){clips=[];selectedClip=null;}
  if(section==='history')sessions=[];
  return false;
 }
}
function loadVideo() {
 const selected=clip(), video=$('#clip-video');
 if(!selected||!video)return;
 videoReady=false;playRequested=false;updatePlayerUi();
 stopPlaybackTicker();
 playbackRequests.request(selected.file_name,url=>{
  if($('#clip-video')!==video)return;
  mediaEvents=new AbortController();
  const options={signal:mediaEvents.signal};
  video.addEventListener('loadeddata',()=>{videoReady=true;$('#media-state-overlay').hidden=true;const item=clip();if(item){item.duration=Number.isFinite(video.duration)?video.duration:null;updateClipMetadata(item);}const specs=$('#clip-video-specs');if(specs)specs.textContent=video.videoWidth&&video.videoHeight?`${video.videoWidth} × ${video.videoHeight}${clip()?.metadata?.fps?' · '+measurement(clip().metadata.fps,1)+' avg recorded FPS':''}`:'';playerDuration=Number.isFinite(video.duration)?video.duration:0;playerPosition=0;pendingSeek=null;pendingSeekTask=Promise.resolve();cancelPendingSeek=null;mediaActionGeneration++;playRequested=false;trimStart=0;trimEnd=playerDuration;updatePlayerUi();stopFilmstrip=renderReplayFilmstrip(video,$('#clip-filmstrip'),$('#clip-preview-status'));},{...options,once:true});
  video.addEventListener('timeupdate',()=>{if(handleTrimBoundary(video))return;if(pendingSeek===null&&Number.isFinite(video.currentTime))playerPosition=clamp(video.currentTime,trimStart,trimEnd||playerDuration);updatePlayerUi();},options);
  video.addEventListener('seeked',()=>{if(pendingSeek===null){playerPosition=video.currentTime;updatePlayerUi();}},options);
  video.addEventListener('play',()=>{playRequested=false;startPlaybackTicker();updatePlayerUi();},options);
  video.addEventListener('pause',()=>{playRequested=false;stopPlaybackTicker();updatePlayerUi();},options);
  video.addEventListener('ended',()=>{stopPlaybackTicker();if(trimEnd>0){playerPosition=trimEnd;updatePlayerUi();seekPlayer(trimStart);}else updatePlayerUi();},options);
  video.addEventListener('error',()=>failVideoLoad(video,'This video could not be played. Retry or open the selected clip externally.'),{...options,once:true});
  watchReplayLoading(video,mediaEvents.signal,()=>failVideoLoad(video,'The video did not finish loading. Retry or open the selected clip externally.'));
  $('#media-state-title').textContent='Loading local video…';
  video.src=url;
 },error=>failVideoLoad(video,message(error)));
}
function failVideoLoad(video,note){
 if($('#clip-video')!==video)return;
 resetPlayback();
 $('#media-state-overlay').hidden=false;$('#media-state-title').textContent='Clip could not load';$('#media-state-detail').textContent=note;
 $('#retry-clip').hidden=false;
 const status=$('#clip-preview-status');status.hidden=false;status.textContent='Frame previews will load when playback is ready.';
 updatePlayerUi();
}
const clamp=(value,min,max)=>Math.max(min,Math.min(max,value));
const timelinePercent=value=>playerDuration>0?clamp(value/playerDuration*100,0,100):0;
function stopPlaybackTicker(){if(playbackFrame!==null){cancelAnimationFrame(playbackFrame);playbackFrame=null;}}
function handleTrimBoundary(video){
 if(video.paused||trimEnd<=0||!Number.isFinite(video.currentTime)||video.currentTime<trimEnd-0.03)return false;
 video.pause();playerPosition=trimEnd;updatePlayerUi();seekPlayer(trimStart);return true;
}
function playbackTick(){
 const video=$('#clip-video');
 if(!video||video.paused||video.ended){playbackFrame=null;return;}
 if(handleTrimBoundary(video)){playbackFrame=null;return;}
 if(pendingSeek===null&&Number.isFinite(video.currentTime)){playerPosition=clamp(video.currentTime,trimStart,trimEnd||playerDuration);updatePlayerUi();}
 playbackFrame=requestAnimationFrame(playbackTick);
}
function startPlaybackTicker(){stopPlaybackTicker();playbackFrame=requestAnimationFrame(playbackTick);}
function seekPlayer(value){
 const video=$('#clip-video');
 if(!video||!playerDuration)return Promise.resolve();
 const target=clamp(Number(value)||0,0,playerDuration);
 const previousCancel=cancelPendingSeek;
 const generation=++seekGeneration;
 if(previousCancel)previousCancel();
 playerPosition=target;pendingSeek=target;
 pendingSeekTask=new Promise(resolve=>{
  let settled=false,timer=null,frameCallback=null;
  const cleanup=()=>{video.removeEventListener('seeked',onSeeked);video.removeEventListener('error',onError);if(timer!==null)clearTimeout(timer);if(frameCallback!==null&&video.cancelVideoFrameCallback)video.cancelVideoFrameCallback(frameCallback);};
  const finish=()=>{if(settled)return;settled=true;cleanup();if(generation===seekGeneration){cancelPendingSeek=null;playerPosition=target;pendingSeek=null;updatePlayerUi();}resolve();};
  const onSeeked=()=>{if(Math.abs(video.currentTime-target)<=.25)finish();};
  const onError=finish;
  const watchFrame=()=>{if(!video.requestVideoFrameCallback)return;frameCallback=video.requestVideoFrameCallback((_,metadata)=>{frameCallback=null;if(Math.abs(video.currentTime-target)<=.25&&Math.abs(metadata.mediaTime-target)<=3)finish();else if(!settled)watchFrame();});};
  cancelPendingSeek=finish;
  video.addEventListener('seeked',onSeeked);video.addEventListener('error',onError,{once:true});watchFrame();timer=setTimeout(finish,1500);
 });
 try{video.currentTime=target;}catch(error){if(cancelPendingSeek)cancelPendingSeek();return Promise.reject(error);}
 updatePlayerUi();return pendingSeekTask;
}
async function playPlayer(video,generation){
 if(!videoReady||video.readyState<1)return;
 const wanted=playerPosition<trimStart||playerPosition>=trimEnd-.03?trimStart:playerPosition;
 const settlingSeek=pendingSeek!==null;
 // Keep the first play() call inside the user gesture. Waiting for an
 // asynchronous seek first makes WebKitGTK treat the request as autoplay.
 const needsSeek=Math.abs(video.currentTime-wanted)>.25;
 if(needsSeek){try{video.currentTime=wanted;}catch(error){throw error;}}
 for(let attempt=0;attempt<2;attempt++){
  try{await video.play();break;}
  catch(error){if(error?.name!=='AbortError'||attempt===1||generation!==mediaActionGeneration)throw error;await new Promise(resolve=>setTimeout(resolve,80));}
 }
 if(generation!==mediaActionGeneration)return;
 if(settlingSeek)await pendingSeekTask;
 if(generation!==mediaActionGeneration)return;
 // WebKitGTK can resolve `seeked` before its paused decoder is ready to resume.
 // Reconcile a seek that was already in flight without losing the original gesture.
 if(Math.abs(video.currentTime-wanted)>.25)await seekPlayer(wanted);
 if(generation!==mediaActionGeneration)return;
}
function setTimelineValue(kind,value,preview=true){if(!videoReady||!playerDuration||exporting)return;const minimumGap=Math.min(.1,playerDuration);if(kind==='start'){trimStart=clamp(value,0,Math.max(0,trimEnd-minimumGap));if(playerPosition<trimStart)playerPosition=trimStart;}else if(kind==='end'){trimEnd=clamp(value,Math.min(playerDuration,trimStart+minimumGap),playerDuration);if(playerPosition>trimEnd)playerPosition=trimEnd;}else playerPosition=clamp(value,trimStart,trimEnd);updatePlayerUi();if(preview)seekPlayer(kind==='start'?trimStart:kind==='end'?Math.max(trimStart,trimEnd-.01):playerPosition);}
function togglePlayback(){
 const video=$('#clip-video');
 if(!video||!videoReady||exporting)return;
 if(!video.paused||playRequested){
  playRequested=false;mediaActionGeneration++;video.pause();updatePlayerUi();return;
 }
 playRequested=true;
 const generation=++mediaActionGeneration;
 updatePlayerUi();
 playPlayer(video,generation).catch(error=>{
  if(generation!==mediaActionGeneration)return;
  playRequested=false;updatePlayerUi();
  notify(error?.name==='AbortError'?'Playback could not resume from the selected position. Try selecting the point again.':message(error));
 });
}
function updatePlayerUi(){
 const seek=$('#player-seek');if(seek){seek.min=String(trimStart);seek.max=String(trimEnd);seek.value=String(pendingSeek??playerPosition);seek.disabled=!videoReady||exporting;updatePrecisionSlider(seek);}
 const video=$('#clip-video');if(!video)return;
 const text=(selector,value)=>{const element=$(selector);if(element&&element.textContent!==value)element.textContent=value;};
 text('#clip-time',`${time(playerPosition)} / ${time(playerDuration||video.duration||0)}`);
 const center=$('#center-play');if(center){const hidden=!videoReady||!video.paused||playRequested;if(center.hidden!==hidden)center.hidden=hidden;center.disabled=exporting;}
 const reset=$('[data-action="reset-trim"]');if(reset)reset.disabled=!videoReady||exporting||(trimStart===0&&trimEnd===playerDuration);
 const toggle=$('#clip-play-toggle');
 if(toggle){
  const paused=video.paused&&!playRequested,label=paused?'Play clip':'Pause clip';
  if(toggle.getAttribute('aria-label')!==label){toggle.innerHTML=icon(paused?'play':'pause');toggle.setAttribute('aria-label',label);toggle.title=label;}
  toggle.disabled=!videoReady||exporting;
 }
 const exportButton=$('#export-clip-button');
 if(exportButton){
  const selectable=videoReady&&playerDuration>0&&trimEnd-trimStart>=.1;
  exportButton.disabled=!native||!selectable||exporting;
  const label=exporting?'Exporting…':'Export selection';
  if(exportButton.textContent.trim()!==label)exportButton.innerHTML=`${icon('download')} ${label}`;
 }
 text('#trim-start-value',videoReady?preciseTime(trimStart):'—');
 text('#trim-end-value',videoReady?preciseTime(trimEnd):'—');
 text('#clip-selected-duration',videoReady?preciseTime(Math.max(0,trimEnd-trimStart)):'—');
 const ruler=$('#clip-time-ruler');
 if(ruler&&ruler.dataset.duration!==String(playerDuration)){
  ruler.dataset.duration=String(playerDuration);
  ruler.querySelectorAll('span').forEach((label,index)=>{label.textContent=playerDuration>0?(playerDuration<6?preciseTime(playerDuration*index/6):time(playerDuration*index/6)):'—';});
 }
 const start=$('#trim-start-bar'),end=$('#trim-end-bar'),playhead=$('#clip-playhead'),selection=$('#trim-selection'),timeline=$('#clip-timeline');
 for(const [element,value,min,max] of [[start,trimStart,0,Math.max(0,trimEnd-.1)],[end,trimEnd,Math.min(playerDuration,trimStart+.1),playerDuration]]){
  if(!element)continue;
  element.disabled=!videoReady||exporting;
  element.style.left=`${timelinePercent(value)}%`;
  element.setAttribute('aria-valuemin',String(min));element.setAttribute('aria-valuemax',String(max));
  element.setAttribute('aria-valuenow',String(value.toFixed(2)));element.setAttribute('aria-valuetext',preciseTime(value));
 }
 if(playhead)playhead.style.left=`${timelinePercent(playerPosition)}%`;
 if(timeline){
  timeline.setAttribute('aria-disabled',String(!videoReady||exporting));
  timeline.setAttribute('aria-valuemax',String(playerDuration));timeline.setAttribute('aria-valuenow',String(playerPosition.toFixed(2)));timeline.setAttribute('aria-valuetext',preciseTime(playerPosition));
  timeline.style.setProperty('--trim-in',`${timelinePercent(trimStart)}%`);timeline.style.setProperty('--trim-out',`${timelinePercent(trimEnd)}%`);
 }
 if(selection){selection.style.left=`${timelinePercent(trimStart)}%`;selection.style.width=`${Math.max(0,timelinePercent(trimEnd)-timelinePercent(trimStart))}%`;}
}

function updateDirtyActionButtons() {
 const discard=$('[data-action="discard-global"]');
 const saveGlobal=$('[data-action="save-global"]');
 const profileChanged=globalProfileChanged(),shortcutsChanged=globalShortcutsChanged();
 // Read retained drafts even when the settings page is not mounted.
 const globalLink=$('#navigation a[href="#global"]');
 if(globalLink){
  const changed=profileChanged||shortcutsChanged;
  globalLink.querySelector('.nav-unsaved-marker').hidden=!changed;
  const label='Global settings'+(changed?', unsaved changes':'');
  globalLink.setAttribute('aria-label',label);globalLink.title=label;
 }
 const dirtyTabs=dirtyGlobalTabs(draft,defaults,shortcutsChanged);
 document.querySelectorAll('.global-tabs [data-global-tab]').forEach(tab=>{const dirty=dirtyTabs[tab.dataset.globalTab];const marker=tab.querySelector('.unsaved-tab-marker');if(marker)marker.hidden=!dirty;tab.setAttribute('aria-label',tab.textContent.replace('●','').trim()+(dirty?', unsaved changes':''));tab.title=dirty?'Unsaved changes':'';});
 const feedback=$('#global-save-feedback');if(feedback){const currentChanged=globalTab==='shortcuts'?shortcutsChanged:profileChanged;feedback.textContent=currentChanged?'Unsaved changes':(profileChanged||shortcutsChanged)?'Changes pending in another tab':'All changes saved';}
 if(discard)discard.disabled=busy||!(profileChanged||shortcutsChanged);
 if(saveGlobal){
  const currentChanged=globalTab==='shortcuts'?shortcutsChanged:profileChanged;
  const invalidCustom=globalTab!=='shortcuts'&&draft.preset==='Custom'&&!draft.metrics.length;
  saveGlobal.disabled=busy||!currentChanged||invalidCustom;
 }
 const saveGame=$('[data-action="save-game"]');
 const gameChanged=gameProfileChanged(game());
 if(saveGame)saveGame.disabled=busy||!gameChanged;
 const discardGame=$('[data-action="discard-game"]');if(discardGame)discardGame.disabled=busy||!gameChanged;
 const gameState=$('#game-save-state');if(gameState)gameState.textContent='Unsaved changes';
 const globalBar=$('.settings-save-bar');if(globalBar)globalBar.hidden=!(profileChanged||shortcutsChanged);
 const gameBar=$('.game-save-bar');if(gameBar)gameBar.hidden=!gameChanged;
 document.body.dataset.unsaved=String(!!$('.change-save-bar:not([hidden])'));
}
function updateObservedElements() {
 const libraryGuide=$('#library-start-guide');if(libraryGuide)libraryGuide.hidden=!showLibraryStartGuide();
 updateDirtyActionButtons();
 refreshUpdateCard();
 for(const el of document.querySelectorAll('[data-module-status]')){
  const value=modules?.[el.dataset.moduleStatus],label=el.dataset.moduleLabel;
  el.textContent=(label?label+' · ':'')+moduleText(value);
  el.className='pill '+moduleClass(value);
 }
 for(const el of document.querySelectorAll('[data-readiness-status]')){const value=modules?.[el.dataset.readinessStatus];el.textContent=value==='Enabled'?'Available':moduleText(value);el.dataset.ready=String(value==='Enabled');}
 const replayDot=$('#replay-status-dot');if(replayDot)replayDot.dataset.status=replayStatusCopy(runtime).statusClass;
 const replayBadge=$('#settings-replay-phase');
 if(replayBadge)replayBadge.textContent=runtime?.phase||'Status unavailable';
 for(const badge of document.querySelectorAll('[data-shortcut-status]')){
  badge.textContent=shortcutText(shortcutStatus?.state);
  badge.className=`pill ${shortcutClass(shortcutStatus?.state)}`;
 }
 const retryShortcuts=$('[data-action="retry-shortcuts"]');
 if(retryShortcuts){retryShortcuts.hidden=!canRetryShortcuts(shortcutStatus,shortcuts);retryShortcuts.disabled=!native||busy;}
 const shortcutFeedback=$('#shortcut-feedback');
 if(shortcutFeedback){const text=shortcutStatus?.message||shortcutStatus?.last_action||'Shortcut assignments are stored locally and monitored by Redunar while the app is running.';if(shortcutFeedback.textContent!==text)shortcutFeedback.textContent=text;}
 const microphone=$('[data-replay-preference="microphone"]');
 if(microphone){microphone.checked=replayPreferences?.microphone_mode!=='off';microphone.disabled=!native||!replayPreferences||busy||Boolean(activeSession?.launch_locked)||active();}
 const microphoneSource=$('[data-microphone-source]');if(microphoneSource){microphoneSource.disabled=!native||!replayPreferences||busy||replayPreferences.microphone_mode==='off'||Boolean(activeSession?.launch_locked)||Boolean(active());enhancePrecisionSelects(microphoneSource.parentElement);}
 const outside=$('[data-replay-preference="outside"]');if(outside)outside.disabled=!native||!replayPreferences||busy;
 const tray=$('[data-preference="tray"]');
 if(tray){tray.checked=preferences.tray===true;tray.disabled=!native||!loaded.preferences||busy;}
 const featureAccess=$('#feature-access');if(featureAccess){featureAccess.value=preferences.betaAccess?'beta':'standard';featureAccess.disabled=!native||!loaded.preferences||busy;enhancePrecisionSelects(featureAccess.parentElement);}
 const automaticUpdates=$('[data-preference="automatic-updates"]');
 if(automaticUpdates){automaticUpdates.checked=preferences.automaticUpdates!==false;automaticUpdates.disabled=!native||!loaded.preferences||busy;}
 const diagnosticLog=$('[data-preference="diagnostic-log"]');
 if(diagnosticLog){diagnosticLog.checked=preferences.diagnosticLog===true;diagnosticLog.disabled=!native||!loaded.preferences||busy;}
 for(const updateAction of document.querySelectorAll('[data-action="check-for-updates"], [data-action="install-update"]'))updateAction.disabled=!native||busy||Boolean(pendingUpdateAction);
 const restart=$('[data-action="restart-redunar"]');if(restart)restart.disabled=!native||busy||Boolean(pendingUpdateAction)||Boolean(active())||Boolean(activeSession?.launch_locked);

 const values={
  'session-timer':time(activeSession?.elapsed_seconds),
  'session-profile':activeSession?.profile_label||'No session profile',
  'session-captures':activeSession?.captures_saved==null?'No session captures':`${activeSession.captures_saved} captures saved`,
  'session-features':activeSession?.feature_summary||'No active session',
  'ram-used':hardware?.ram_used_bytes==null?'—':(hardware.ram_used_bytes/1073741824).toFixed(1)+' GiB',
  'ram-available':hardware?.ram_used_bytes!=null&&hardware?.ram_total_bytes!=null?`${Math.max(0,(hardware.ram_total_bytes-hardware.ram_used_bytes)/1073741824).toFixed(1)} GiB available`:'',
  'vram-available':hardware?.vram_used_bytes!=null&&hardware?.vram_total_bytes!=null?`${Math.max(0,(hardware.vram_total_bytes-hardware.vram_used_bytes)/1073741824).toFixed(1)} GiB available`:'',
  'ram-total':hardware?.ram_total_bytes==null?'Capacity unavailable':`of ${(hardware.ram_total_bytes/1073741824).toFixed(1)} GiB`,
  'vram-total':hardware?.vram_total_bytes==null?'Capacity unavailable':`of ${(hardware.vram_total_bytes/1073741824).toFixed(1)} GiB`,
  'overview-chart-label':overviewMetric==='fps'?'Frame rate':'Frame time',
  'GPU-temp':hardware?.gpu_temperature_celsius==null?'—':measurement(hardware.gpu_temperature_celsius)+'°C',
  'CPU-temp':hardware?.cpu_temperature_celsius==null?'—':measurement(hardware.cpu_temperature_celsius)+'°C',
  'GPU-load':measurement(hardware?.gpu_utilization_percent), 'CPU-load':measurement(hardware?.cpu_utilization_percent),
  'GPU-model':hardware?.gpu_model||'Unavailable', 'CPU-model':hardware?.cpu_model||'Unavailable',
  'GPU-clock':hardware?.gpu_clock_mhz==null?'Clock —':`${measurement(hardware.gpu_clock_mhz)} MHz`,
  'CPU-clock':'Clock —',
  'vram-used':hardware?.vram_used_bytes==null?'—':(hardware.vram_used_bytes/1073741824).toFixed(1)+' GiB',
  'session-note':activeSession?.message||(active()?'':'Launch a saved game from Library or a configured game from Steam to begin a session.'),
  'live-phase':activeSession?.phase==='Ended'?'Session ended':activeSession?.measurements?.phase||'Awaiting telemetry',
  'session-name':activeSession?.game||'No active game', 'session-phase':activeSession?.phase||'Unavailable',
  'replay-status-title':replayStatusCopy(runtime).title, 'replay-phase':runtime?.can_save?'':replayStatusCopy(runtime).detail, 'replay-buffer':runtime?measurement(runtime.buffered_seconds)+' s buffered':'—', 'replay-frame-count':runtime?`${runtime.received_frame_count||0} received · ${runtime.encoded_packet_count||0} encoded · ${['Buffering','Saving'].includes(runtime.phase)?runtime.audio_active?'audio active':runtime.audio_packet_count>0?'audio stalled':'audio waiting':'audio inactive'}`:'—',
  'overview-replay-status':runtime?`${runtime.phase} · ${measurement(runtime.buffered_seconds)} s buffered`:'Replay status unavailable',
 };
 for(const [id,text] of Object.entries(values)){ const el=document.getElementById(id);if(el&&el.textContent!==text)el.textContent=text; }
 for(const name of ['GPU','CPU']) { const el=$(`#${name}-meter`);if(el)el.style.width=`${Math.max(0,Math.min(100,hardware?.[name.toLowerCase()+'_utilization_percent']||0))}%`; }
 for(const [id,value] of [['GPU-temperature',hardware?.gpu_temperature_celsius],['CPU-temperature',hardware?.cpu_temperature_celsius],['ram',hardware?.ram_total_bytes?hardware.ram_used_bytes/hardware.ram_total_bytes*100:0],['vram',hardware?.vram_total_bytes?hardware.vram_used_bytes/hardware.vram_total_bytes*100:0]]){const bar=document.getElementById(id+'-meter');if(bar)bar.style.width=`${clamp(value||0,0,100)}%`;}
 const shortcut=$('#save-replay-shortcut');
 if(shortcut){
  const assignment=savedShortcutMap[$('#save-duration')?.value]||'';
  shortcut.textContent=assignment;shortcut.hidden=!assignment;
  const button=$('[data-action="save-replay"]');
  if(button){const duration=Number($('#save-duration')?.value)||30;button.title=`Save last ${duration} seconds · ${assignment?'Saved shortcut: '+assignment:'No shortcut assigned'}`;}
  // Reserve the longest saved assignment across durations so selecting an
  // unassigned duration doesn't move the adjacent save controls.
  const slot=shortcut.parentElement;
  const characters=Math.max(0,...durations.map(duration=>String(savedShortcutMap[duration]||'').length));
  slot.hidden=characters===0;
  slot.style.setProperty('--shortcut-characters',String(characters));
 }
 const summary=$('#session-capture-summary');if(summary){const content=captureSummary(runtime);if(summary.innerHTML!==content)summary.innerHTML=content;}
 const end=$('[data-action="end-session"]');if(end)end.hidden=!active();
 const save=$('[data-action="save-replay"]');if(save)save.disabled=!native||busy||!runtime?.can_save;
 const launch=$('[data-action="launch-game"]');
 if(launch){
  const status=installation.status(game()?.id);
  launch.disabled=!native||busy||Boolean(activeSession?.launch_locked)||active()||status!=='installed';
  const note=$('#game-installation-note');
  note.textContent=status==='not_installed'?'Not installed':status==='unknown'?'Installation status unavailable':status==='checking'?'Checking installation…':'';
  note.hidden=status==='installed';
 }
 const observed=activeSession?.measurements;
 for(const [id,value] of Object.entries({'live-fps':observed?.average_fps,'live-frame-time':observed?.frame_time_ms,'live-low':observed?.one_percent_low_fps,'live-lowest':observed?.point_one_percent_low_fps})){
  const el=document.getElementById(id);if(el)el.firstChild.nodeValue=measurement(value,id==='live-frame-time'?1:0);
 }
 const plot=$('#live-chart');
 const currentGame=activeSession?.game||'';
 if(liveTimeline.game!==currentGame){liveTimeline={game:currentGame,revision:null,values:[],times:[]};}
 const revision=observed?.revision??null;
 if(revision!==null&&revision!==liveTimeline.revision){
  liveTimeline.revision=revision;
  const newest=(observed?.frame_intervals_ns||[]).findLast(n=>Number.isFinite(n)&&n>0);
  if(newest!==undefined){liveTimeline.values.push(newest/1e6);liveTimeline.times.push(performance.now());if(liveTimeline.values.length>LIVE_TIMELINE_LIMIT){liveTimeline.values.shift();liveTimeline.times.shift();}}
 }
 if(plot){
  const renderedRevision=`${liveTimeline.game}/${liveTimeline.revision??'empty'}/${liveTimeline.values.length}/${overviewMetric}`;
  if(plot.dataset.revision!==renderedRevision){plot.dataset.revision=renderedRevision;plot.innerHTML=chart(overviewMetric==='fps'?liveTimeline.values.map(value=>1000/value):liveTimeline.values,overviewMetric==='fps'?'FPS':'ms','recent frame-time timeline');}
 }
}
const refreshRuntime=latestRuntimeRefresh(readRuntime);
async function readRuntime() {
 if(!native)return;
 const slowRefresh=Date.now()-lastSlowRuntimeRefresh>=10000;
 if(slowRefresh)lastSlowRuntimeRefresh=Date.now();
 const slowResults=slowRefresh?Promise.allSettled([call('diagnostics_snapshot'),call('replay_display_capability')]):null;
 const results=await Promise.allSettled([call('daemon_status'),call('replay_runtime_status'),call('session_status'),call('module_statuses'),call('shortcut_status')]);
 const oldRevision=runtime?.completed_save_revision;
 const oldHistory=lastLoadedHistoryRevision;
 const oldDisplay=JSON.stringify(displayCapability);
 hardware=results[0].status==='fulfilled'&&results[0].value.available?results[0].value:null;
 preferences.diagnosticLogStatus=results[0].status==='fulfilled'?results[0].value.diagnostic_log_status:undefined;
 runtime=results[1].status==='fulfilled'?results[1].value:null;
 activeSession=results[2].status==='fulfilled'?visibleSession(results[2].value):null;
 modules=results[3].status==='fulfilled'?results[3].value:null;
 shortcutStatus=results[4].status==='fulfilled'?results[4].value:shortcutStatus;
 if(slowResults){
  const [diagnosticsResult,displayResult]=await slowResults;
  diagnostics=diagnosticsResult.status==='fulfilled'?diagnosticsResult.value:diagnostics;
  displayCapability=displayResult.status==='fulfilled'?displayResult.value:displayCapability;
 }
 const connectionStatus=$('#connection-status');
 if(connectionStatus)connectionStatus.textContent=results[0].value?.read_only?'Another Redunar app is open · read-only':results.slice(0,4).every(r=>r.status==='rejected')?'Backend unavailable':'Production backend';
 updateObservedElements();
 const logStatus=$('#diagnostic-log-status');
 if(logStatus)logStatus.textContent=diagnosticDisplayStatus();
 if(route()==='global'&&JSON.stringify(displayCapability)!==oldDisplay)render();
 if(activeSession?.history_revision!==undefined&&activeSession.history_revision!==oldHistory){
  const loadedHistory=await reloaders['reload-history']();
  if(loadedHistory&&route()==='history')render();
 }
 if(oldRevision!==undefined&&runtime?.completed_save_revision>oldRevision){
  await reloaders['reload-clips']();
  notify('Replay saved to your local clip store.');
  if(route()==='replay'&&!$('#clip-video')?.paused)return;
  render();
 }
}
async function checkUpdatesOnStartup() {
 if(!native||startupUpdateCheckStarted||preferences.automaticUpdates!==true)return;
 startupUpdateCheckStarted=true;
 try {
  const status=await call('check_for_updates');
  updateStatus=status;
  if(status.state==='available')notify('A Redunar update is available. Open Settings to install it.');
  if(status.state==='handoff')notify('A Redunar update needs installation confirmation. Open Settings for details.');
  if(status.state==='restart-needed')notify('A Redunar update is installed. Fully quit and reopen Redunar to load it.');
 } catch(error) {
  updateStatus={current_version:'',state:'error',message:message(error),latest_version:null,package_kind:null,asset_name:null};
 }
}
function pollDelay() {
 if(document.hidden)return 2000;
 return ['overview','replay'].includes(route())?1000:2500;
}
async function poll() {
 try { if(!document.hidden)await refreshRuntime(); } catch(error) { notify(message(error)); }
 setTimeout(poll,pollDelay()); // Schedule after completion; slow reads never build a queue.
}
function reportActionError(error) {
 const feedback=$('#dialog-error');
 if($('#dialog').open){feedback.textContent=message(error);feedback.hidden=false;feedback.focus();}
 else notify(message(error));
}

function showTrashConfirmation(item) {
 if(busy||!item)return;
 modal('Move to trash bin?',`<form id="trash-clips-form" class="compact-clip-form"><input type="hidden" name="fileNames" value="${escape(JSON.stringify([item.id]))}"><div class="trash-confirm-content"><strong>${escape(clipTitle(item))}</strong><p>Trashed clips can be restored from the trash bin.</p></div><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Move to trash bin</button></div></form>`);
}
async function restoreTrashedClips(fileNames) {
 if(!fileNames.length)return;
 perform(async()=>{await call('restore_replay_clips',{fileNames});lastTrashed=lastTrashed.filter(id=>!fileNames.includes(id));if(!lastTrashed.length){clearTimeout(clipTrashFeedbackTimer);clipTrashFeedbackTimer=null;}await reloaders['reload-clips']();if($('#dialog').open)$('#dialog').close();render();notify('Clips restored.');});
}

async function perform(action) {
 if(busy)return;
 busy=true;document.body.classList.add('native-busy');updateObservedElements();
 $('#dialog-error').hidden=true;$('#dialog-error').textContent='';
 try { await action(); }
 catch(error){ reportActionError(error); }
 finally { busy=false;document.body.classList.remove('native-busy');updateObservedElements(); }
}
function showExportProgress(progress=0) {
 const body=$('#dialog-body');if(!body)return;
 body.innerHTML=`<p>Creating a separate MP4 from the selected range. The original recording stays unchanged.</p><div class="clip-export-progress"><div><span>Exporting clip</span><output id="clip-export-percent">${Math.round(progress)}%</output></div><progress id="clip-export-progress" max="100" value="${progress}"></progress></div><button class="button" type="button" data-export-cancel>Cancel export</button>`;
 const close=$('#dialog [data-close]');if(close)close.disabled=true;
}
async function pollClipExport() {
 if(!exporting)return;
 try {
  const status=await call('clip_export_status');
  const progress=clamp(Number(status.progress)||0,0,100),bar=$('#clip-export-progress'),label=$('#clip-export-percent');
  if(bar)bar.value=progress;if(label)label.textContent=`${Math.round(progress)}%`;
 } catch {}
 if(exporting)exportPollTimer=setTimeout(pollClipExport,250);
}
async function runClipExport(fileName,start,end,outputName,destination) {
 if(exporting||busy)return;
 exporting=true;busy=true;updatePlayerUi();showExportProgress();pollClipExport();
 try {
  const result=await call('export_replay_clip',{fileName,startSeconds:start,endSeconds:end,outputName});
  await reloaders['reload-clips']();selectedClip=result.file_name;
  $('#dialog').close();render();modal('Clip exported',`<div class="export-complete"><strong>${escape(result.file_name)}</strong><p>${escape(result.destination||destination)}</p><small>${preciseTime(result.duration_seconds)} · ${(result.bytes/1048576).toFixed(1)} MB</small></div><div class="dialog-actions"><button class="button" data-action="show-export-folder" data-file-name="${escape(result.file_name)}">Show in folder</button><button class="button primary" data-action="open-exported-clip" data-file-name="${escape(result.file_name)}">Open clip</button><button class="button" data-close>Done</button></div>`);
 } catch(error) {
  $('#dialog').close();modal('Export failed',`<p>${escape(message(error))}</p><p class="small-note">Your original recording is unchanged.</p><form id="trim-export-form"><input type="hidden" name="fileName" value="${escape(fileName)}"><input type="hidden" name="start" value="${start}"><input type="hidden" name="end" value="${end}"><label class="form-label">Filename<input name="outputName" required maxlength="200" value="${escape(outputName)}"></label><div class="dialog-actions"><button class="button" data-close type="button">Cancel</button><button class="button primary" type="submit">Retry export</button></div></form>`);
 } finally {
  exporting=false;busy=false;clearTimeout(exportPollTimer);exportPollTimer=null;
  const close=$('#dialog [data-close]');if(close)close.disabled=false;
  updateObservedElements();updatePlayerUi();
 }
}
const reloaders={
 'reload-games':()=>readSection('catalog','catalog_games',acceptGames),
 'reload-clips':async()=>{await readSection('clips','replay_clips',acceptClips);try{storageStatus=await call('replay_storage_status');delete errors.storage;}catch(error){storageStatus=null;errors.storage=message(error);}},
 'reload-history':async()=>{const loadedHistory=await readSection('history','session_history',data=>{sessions=mapSessions(data);selectedSession=0;historyCursor=null;loaded.history=true;});if(loadedHistory&&activeSession)lastLoadedHistoryRevision=activeSession.history_revision;return loadedHistory;},
 'reload-global':()=>readSection('global','global_settings',acceptGlobal),
 'reload-preferences':()=>readSection('preferences','app_preferences',data=>{preferences.tray=data.close_to_tray;preferences.automaticUpdates=data.automatic_updates!==false;preferences.diagnosticLog=data.diagnostic_log===true;preferences.diagnosticLogPath=data.diagnostic_log_path||null;preferences.diagnosticLogStatus=data.diagnostic_log_status;preferences.betaAccess=data.beta_access===true;loaded.preferences=true;}),
 'reload-replay-preferences':()=>readSection('replayPreferences','replay_preferences',data=>{replayPreferences=data;loaded.replayPreferences=true;}),
 'reload-microphone-devices':async()=>{try{microphoneDevices=await call('microphone_inputs');delete errors.microphoneDevices;}catch(error){microphoneDevices=[];errors.microphoneDevices=message(error);}},
 'reload-diagnostics':async()=>{try{diagnostics=await call('diagnostics_snapshot');delete errors.diagnostics;}catch(error){diagnostics=null;errors.diagnostics=message(error);}},
};
document.addEventListener('pointerup',event=>{
 const toggle=event.target.closest?.('[data-player="toggle"]');
 if(!toggle||toggle.disabled)return;
 event.preventDefault();
 suppressPlayerToggleClickUntil=performance.now()+500;
 togglePlayback();
});
document.addEventListener('pointerdown',event=>{
 const resizeHandle=event.target.closest?.('[data-resize-direction]');
 if(!resizeHandle||!appWindow||event.button!==0)return;
 event.preventDefault();
 appWindow.startResizeDragging(resizeHandle.dataset.resizeDirection).catch(error=>notify(`Window resize failed: ${message(error)}`));
});
document.addEventListener('click',event=>{
 const windowControl=event.target.closest('[data-window-action]');
 if(windowControl){
  if(!appWindow)return;
  const action=windowControl.dataset.windowAction;
  const request=action==='minimize'?appWindow.minimize():action==='maximize'?appWindow.toggleMaximize():appWindow.close();
  request.catch(error=>notify(`Window action failed: ${message(error)}`));
  return;
 }
 const timeline=event.target.closest('#clip-timeline');
 if(timeline){if(suppressTimelineClick){suppressTimelineClick=false;return;}if(!event.target.closest('.timeline-handle')){const value=pointerTimelineValue(event);if(value!==null)setTimelineValue('seek',value,true);return;}}
 const more=event.target.closest('.game-more');document.querySelectorAll('.game-more[open]').forEach(el=>{if(el!==more)el.open=false;});
 const target=event.target.closest('button,a');if(!target)return;
 if(more&&target.matches('button'))more.open=false;
 if(target.classList.contains('skip-link')){event.preventDefault();workspace.focus();return;}
 if(target.hasAttribute('data-export-cancel')){target.disabled=true;target.textContent='Cancelling…';call('cancel_clip_export').catch(error=>notify(message(error)));return;}
 if(target.hasAttribute('data-close')){if(!exporting)closePrecisionSelect();$('#dialog').close();return;}
 if(target.hasAttribute('data-pick-folder')){const input=target.closest('form').elements.namedItem('parent');target.disabled=true;call('pick_replay_folder').then(path=>{if(path)input.value=path;}).catch(error=>notify(message(error))).finally(()=>{target.disabled=false;if(target.isConnected)target.focus({preventScroll:true});});return;}
 if(target.dataset.pickExecutable){
  if(!native)return;
  const input=target.closest('form')?.elements?.namedItem(target.dataset.pickExecutable);
  target.disabled=true;
  call('pick_executable').then(path=>{if(path&&input)input.value=path;}).catch(error=>notify(message(error))).finally(()=>{target.disabled=false;});
  return;
 }
 if(target.dataset.copySteamOptions){
  const value=target.dataset.copySteamOptions;
  if(!value)return;
  const copy=async()=>{
   if(navigator.clipboard?.writeText){await navigator.clipboard.writeText(value);return;}
   const helper=document.createElement('textarea');helper.value=value;helper.setAttribute('readonly','');helper.style.position='fixed';helper.style.opacity='0';document.body.appendChild(helper);helper.select();
   try{if(!document.execCommand('copy'))throw new Error('Clipboard access was unavailable. Select the value and copy it manually.');}
   finally{helper.remove();}
  };
  copy().then(()=>notify('Steam launch options copied.')).catch(error=>notify(message(error)));
  return;
 }
 if(target.dataset.deleteClip||target.dataset.action==='delete-selected-clip'){
  const item=target.dataset.deleteClip?clips.find(c=>c.id===target.dataset.deleteClip):clip(); if(!item)return;
  showTrashConfirmation(item);return;
 }
 if(target.dataset.recentClip){selectedClip=target.dataset.recentClip;return;}
 if(target.dataset.action==='reload-microphone-devices'){perform(async()=>{await reloaders['reload-microphone-devices']();render();});return;}
 if(target.dataset.favoriteClip||target.dataset.action==='favorite-selected-clip'){const item=target.dataset.favoriteClip?clips.find(c=>c.id===target.dataset.favoriteClip):clip();if(item)saveClipDetails(item,{favorite:!item.favorite}).catch(error=>notify(message(error)));return;}
 if(target.dataset.clipScope){clipFilters.favorites=target.dataset.clipScope==='favorites';refreshClipRail();return;}
 if(target.dataset.removeClipFilter){clipFilters[target.dataset.removeClipFilter]='all';syncClipFilterFields();refreshClipRail();return;}
 if(target.dataset.action==='clear-clip-filters'){clipFilters={search:'',game:'all',date:'all',sort:'newest',favorites:false};const search=$('#clip-search');if(search)search.value='';syncClipFilterFields();refreshClipRail();return;}
 if(target.dataset.action==='edit-clip-filename'){const c=clip();if(c)modal('Edit filename',`<form id="clip-filename-form"><label class="form-label">Filename<input name="newFileName" required maxlength="240" value="${escape(clipFilenameStem(c.file_name))}"></label><input type="hidden" name="fileName" value="${escape(c.file_name)}"><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Rename file</button></div></form>`);return;}
 if(target.dataset.clip){selectClip(target.dataset.clip);return;}
 if(target.dataset.action==='retry-clip'){reloadSelectedClip();return;}
 if(target.dataset.game){selectedGame=target.dataset.game;render();return;}
 if(target.dataset.session!==undefined){selectedSession=Number(target.dataset.session);historyCursor=null;historyCompare=false;historyPeerId='';render();return;}
 if(target.dataset.overviewView){overviewMetric=target.dataset.overviewView;document.querySelectorAll('[data-overview-view]').forEach(el=>{const selected=el.dataset.overviewView===overviewMetric;el.classList.toggle('active',selected);el.setAttribute('aria-pressed',String(selected));});updateObservedElements();return;}
 if(target.dataset.historyView){historyMetric=target.dataset.historyView;render();return;}
 if(target.dataset.action==='history-compare'){historyCompare=!historyCompare;render();return;}
 if(target.dataset.preset){if(busy||!loaded.global)return;const next=target.dataset.preset;if(next==='Custom'&&draft.preset!=='Custom')draft.metrics=effectiveMetrics(draft.preset||'Compact',draft.metrics);draft.preset=next;syncMetricToggles();updateHud();return;}
 if(target.dataset.layout){if(busy||!loaded.global)return;draft.layout=target.dataset.layout;updateHud();return;}
 if(target.dataset.palette){if(busy||!loaded.global)return;draft.palette=target.dataset.palette;updateHud();return;}
 if(target.dataset.globalTab){globalTab=target.dataset.globalTab;if(route()==='global')render();return;}
 if(target.dataset.libraryTab){libraryTab=target.dataset.libraryTab;render();return;}
 if(target.dataset.action==='reset-trim'){if(videoReady&&!exporting){trimStart=0;trimEnd=playerDuration;seekPlayer(0);updatePlayerUi();}return;}
 if(target.dataset.player){const video=$('#clip-video');if(video){if(target.dataset.player==='toggle'){if(performance.now()<suppressPlayerToggleClickUntil){suppressPlayerToggleClickUntil=0;return;}togglePlayback();}if(target.dataset.player==='back')seekPlayer(clamp((pendingSeek??playerPosition)-5,trimStart,trimEnd));if(target.dataset.player==='forward'){const next=clamp((pendingSeek??playerPosition)+5,trimStart,trimEnd);if(next>=trimEnd-.01&& !video.paused)video.pause();seekPlayer(next);}updatePlayerUi();}return;}
 const action=target.dataset.action;
 if(action==='clear-history-filters'){historyFilters={search:'',game:'all',date:'all'};render();return;}
 if(action==='undo-clip-trash'){restoreTrashedClips(lastTrashed);return;}
 if(action==='view-clip-trash'){perform(async()=>{const items=await call('trashed_replay_clips');modal('Trash bin',items.length?`<p>Restore clips to Saved clips.</p><div class="trashed-clip-list">${items.map(c=>`<div><strong>${escape(c.file_name)}</strong><button class="button" data-action="restore-trash-clip" data-file-name="${escape(c.file_name)}">Restore</button></div>`).join('')}</div>`:'<p>The trash bin is empty.</p>');});return;}
 if(action==='restore-trash-clip'){restoreTrashedClips([target.dataset.fileName]);return;}
 if(action==='show-export-folder'){perform(()=>call('open_replay_folder'));return;}
 if(action==='open-exported-clip'){selectedClip=target.dataset.fileName;$('#dialog').close();render();return;}
 if(action==='open-updates'){event.preventDefault();if(route()==='settings'){$('#software-updates').scrollIntoView({block:'nearest'});$('#software-updates').focus({preventScroll:true});}else{focusUpdatesOnRender=true;location.hash='settings';}return;}
 if(action==='restart-redunar'){requestUpdateRestart();return;}
 if(action==='save-and-restart'||action==='discard-and-restart'){finishDraftRestart(action==='save-and-restart');return;}
 if(action==='clear-shortcuts'){draftShortcuts=Array(9).fill('');render();return;}
 if(!action)return;
 if(action==='retry-shortcuts'){
  perform(async()=>{try{shortcutStatus=await call('activate_shortcuts');notify('Shortcut activation requested.');}finally{shortcutStatus=await call('shortcut_status').catch(()=>shortcutStatus);updateObservedElements();}});return;
 }
 if(action==='check-for-updates'){
  performUpdate('check_for_updates','checking',{refreshPending:true});return;
 }
 if(action==='install-update'){
  performUpdate('install_update','installing');return;
 }
 if(action==='discard-global'){if(!globalProfileChanged()&&!globalShortcutsChanged())return;draft=structuredClone(defaults);draftShortcuts=[...shortcuts];render();return;}
 if(action==='discard-game'){game().overrides={...savedGameOverrides.get(game().id)};game().revision=game().savedRevision??game().revision;render();return;}
 if(action==='reset-game'){
  delete game().overrides.overlay;delete game().overrides.replay;delete game().overrides.captureMetrics;
  render();notify('Draft now uses global defaults. Save game settings to apply.');return;
 }
 if(action==='add-game'){
  modal('Add a local game','<form id="add-game-form"><label class="form-label">Game name<input name="name" required maxlength="70" placeholder="Game name"></label><label class="form-label">Executable path<div class="picker-field"><input name="exe" required maxlength="4096" placeholder="/absolute/path/to/game"><button class="button" type="button" data-pick-executable="exe">Choose file</button></div></label><p class="small-note">Adds a verified local executable to the local library. It does not launch the game.</p><button class="button primary" type="submit">Add game</button></form>');return;
 }
 if(action==='edit-launch'){
  const g=game(); if(!g)return;
   modal('Edit launch settings',`<form id="edit-launch-form"><input type="hidden" name="gameId" value="${escape(g.id)}"><input type="hidden" name="expectedRevision" value="${escape(g.launch_revision)}"><label class="form-label">Executable path<div class="picker-field"><input name="executable" required maxlength="4096" value="${escape(g.exe)}"><button class="button" type="button" data-pick-executable="executable">Choose file</button></div></label><label class="form-label">Arguments <span class="small-note">One argument per line; values are passed directly.</span><textarea name="arguments" rows="4" maxlength="8192">${escape((g.args||[]).join('\n'))}</textarea></label><label class="form-label">Working directory<input name="workingDirectory" maxlength="4096" value="${escape(g.workdir||'')}"></label><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Save launch settings</button></div></form>`);return;
 }
 if(action==='remove-game'){
  const g=game(); if(!g)return;
  modal('Remove game from library',`<p>Remove <strong>${escape(g.name)}</strong> from the local catalog?</p><p class="small-note">This removes its saved profile and launch match. Existing clips and session history remain untouched.</p><form id="remove-game-form"><input type="hidden" name="gameId" value="${escape(g.id)}"><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Remove game</button></div></form>`);return;
 }
 if(action==='check-steam-setup'){
  if(!native)return;refreshSteamSetup($('.steam-setup-card'),game()?.id,call);return;
 }
 if(action==='scan-games'){
  perform(async()=>{discoveryModal(await call('discover_games'));});return;
 }
 if(action==='export'){
  const selected=clip(),duration=Math.max(0,trimEnd-trimStart);if(!selected||duration<.1)return;
  mediaActionGeneration++;playRequested=false;$('#clip-video')?.pause();updatePlayerUi();
  modal('Export clip',`<form id="trim-export-form" class="compact-clip-form"><input type="hidden" name="fileName" value="${escape(selected.file_name)}"><input type="hidden" name="start" value="${trimStart}"><input type="hidden" name="end" value="${trimEnd}"><label class="form-label">Filename<input name="outputName" required maxlength="200" value="${escape(clipFilenameStem(selected.file_name)+'-trimmed')}"></label><div class="export-folder-field"><span>Save to</span><div class="export-folder-display" title="${escape(replayPreferences?.resolved_directory||'Replay folder')}">${icon('folder')}<span id="export-destination">${escape(replayPreferences?.resolved_directory||'Replay folder')}</span></div></div><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Export</button></div></form>`);return;
 }
 perform(async()=>{
  if(reloaders[action]){await reloaders[action]();render();return;}
  if(action==='launch-game'){
   try { await call('launch_game',{gameId:game().id}); }
   catch(error){installation.refresh();modal('Game could not start',`<p class="small-note">${escape(message(error))}</p>`);return;}
   await refreshRuntime();render();
   notify('Launch requested. The session supervisor will report actual capture and replay status.');
  }
  if(action==='folder')await call('open_replay_folder');
  if(action==='open-clip-external'){if(!clip())throw new Error('Select a clip first.');await call('open_clip_external',{fileName:clip().file_name});notify('Opened the selected clip in your desktop player.');}
  if(action==='open-diagnostic-log-folder')await call('open_diagnostic_log_folder');
  if(action==='change-replay-folder'){
   modal('Choose replay folder',`<form id="replay-folder-form"><label class="form-label">Parent directory<div class="picker-field"><input name="parent" aria-label="Parent directory" required maxlength="4096" value="${escape(replayPreferences?.custom_save_parent||'')}" placeholder="/home/you/Videos"><button class="button" type="button" data-pick-folder>Choose folder</button></div></label><p class="small-note">Redunar will create and own a <code>Redunar Replays</code> folder inside this directory. Existing clips stay where they are.</p><div class="dialog-actions"><button class="button" type="button" data-close>Cancel</button><button class="button primary" type="submit">Save folder</button></div></form>`);
  }
  if(action==='reset-replay-folder'){
   replayPreferences=await call('set_replay_save_parent',{parent:null});clearClipTrashFeedback();await reloaders['reload-clips']();render();notify('Replay folder reset to the default location.');
  }
  if(action==='save-game'){
   const saved=await saveGameProfileDraft(game()?.id);if(!saved)return;
   render();notify(saved.liveNotice||'Game settings saved.');
  }
  if(action==='save-global'){
   if(globalTab==='shortcuts'){
    const notice=await saveShortcutDraft();if(notice)notify(notice);
   }else{
    const saved=await saveGlobalProfileDraft();if(saved)notify(saved.liveNotice||'Global defaults saved. Shortcut changes have their own Save action.');
   }
   render();
  }
  if(action==='save-replay'){
   await call('save_replay',{durationSeconds:Number($('#save-duration').value)});
   notify('Replay save requested. The clip will appear when the backend finishes saving.');await refreshRuntime();
  }
  if(action==='end-session'){await call('end_session');await refreshRuntime();notify(activeSession?.message||'Session ended by the backend.');render();}
 });
});
document.addEventListener('input',event=>{
 const t=event.target;
 if(t.id==='player-seek'){seekPlayer(Number(t.value));return;}
 if(t.id==='clip-search'){clipFilters.search=t.value;refreshClipRail();}
 if(t.id==='game-search'){$('#library-list').innerHTML=libraryRows(t.value);loadGameArtwork(workspace,call);}
 if(t.id==='history-search'){historyFilters.search=t.value;refreshHistoryRail();}
 if(t.dataset.historyCursor!==undefined)updateHistoryCursor(t.value);
 if(t.dataset.global){draft[t.dataset.global]=t.type==='checkbox'?t.checked:['fps','scale','opacity'].includes(t.dataset.global)?Number(t.value):t.value;updateHud();}
 if(t.dataset.metric){draft.metrics=t.checked?[...new Set([...draft.metrics,t.dataset.metric])]:draft.metrics.filter(m=>m!==t.dataset.metric);draft.preset='Custom';const preset=$('[data-global="preset"]');if(preset){preset.value='Custom';enhancePrecisionSelects(workspace);}syncMetricToggles();updateHud();}
 if(t.dataset.shortcut!==undefined){draftShortcuts[Number(t.dataset.shortcut)]=t.value;updateDirtyActionButtons();}

});
function pointerTimelineValue(event){const timeline=$('#clip-timeline');if(!timeline||!playerDuration)return null;const rect=timeline.getBoundingClientRect();return clamp((event.clientX-rect.left)/Math.max(1,rect.width)*playerDuration,0,playerDuration);}
document.addEventListener('pointerdown',event=>{const timeline=event.target.closest('#clip-timeline');if(!timeline||!videoReady||exporting)return;event.preventDefault();suppressTimelineClick=true;timelineDrag=event.target.closest('[data-timeline]')?.dataset.timeline||'seek';timeline.setPointerCapture(event.pointerId);const value=pointerTimelineValue(event);if(value!==null)setTimelineValue(timelineDrag,value,false);});
document.addEventListener('pointermove',event=>{if(!timelineDrag)return;const value=pointerTimelineValue(event);if(value!==null)setTimelineValue(timelineDrag,value,false);});
document.addEventListener('pointerup',event=>{if(!timelineDrag)return;const kind=timelineDrag,value=pointerTimelineValue(event);timelineDrag=null;if(value!==null)setTimelineValue(kind,value,true);});
document.addEventListener('pointercancel',()=>{timelineDrag=null;suppressTimelineClick=false;});
document.addEventListener('keydown',event=>{const shortcut=event.target.closest('[data-shortcut]');if(shortcut){event.preventDefault();if(event.key==='Escape'){shortcut.value='';draftShortcuts[Number(shortcut.dataset.shortcut)]='';updateDirtyActionButtons();return;}if(['Control','Alt','Shift','Meta'].includes(event.key))return;const keyMap={' ':'Space','ArrowUp':'Up','ArrowDown':'Down','ArrowLeft':'Left','ArrowRight':'Right','Escape':'Esc','Tab':'Tab','Enter':'Enter','Backspace':'Backspace','Delete':'Delete'};const key=keyMap[event.key]||(/^F\d{1,2}$/i.test(event.key)?event.key.toUpperCase():event.key.length===1?event.key.toUpperCase():event.key);const parts=[];if(event.ctrlKey)parts.push('Ctrl');if(event.altKey)parts.push('Alt');if(event.shiftKey)parts.push('Shift');if(event.metaKey)parts.push('Super');parts.push(key);const value=parts.join('+');shortcut.value=value;draftShortcuts[Number(shortcut.dataset.shortcut)]=value;updateDirtyActionButtons();return;}const target=event.target.closest('[data-timeline]');if(!target||!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;event.preventDefault();const kind=target.dataset.timeline,current=kind==='start'?trimStart:kind==='end'?trimEnd:playerPosition;let value=current+(event.key==='ArrowRight'?1:-1)*(event.shiftKey?5:.25);if(event.key==='Home')value=kind==='seek'?trimStart:kind==='end'?trimStart+.1:0;if(event.key==='End')value=kind==='seek'?trimEnd:kind==='start'?trimEnd-.1:playerDuration;setTimelineValue(kind,value,true);});
document.addEventListener('change',event=>{
 if(event.target.id==='feature-access'){
  const selected=event.target.value,requested=selected==='beta';
  event.target.value=preferences.betaAccess?'beta':'standard';
  if(!['standard','beta'].includes(selected)||!native||!loaded.preferences||busy||requested===preferences.betaAccess){enhancePrecisionSelects(event.target.parentElement);return;}
  perform(async()=>{
   const saved=await call('set_beta_access',{enabled:requested});
   preferences.betaAccess=saved.beta_access===true;
   notify(preferences.betaAccess?'Beta access enabled. Restart Redunar before trying beta features.':'Beta access disabled. Restart Redunar to return to standard features.');
   render();
  }).then(()=>focusPrecisionSelect($('#feature-access')));
  return;
 }
 const t=event.target;
 if(t.dataset.historyFilter){historyFilters[t.dataset.historyFilter]=t.value;if(refreshHistoryRail())focusPrecisionSelect($(`[data-history-filter="${t.dataset.historyFilter}"]`));return;}
 if(t.id==='comparison-session'){historyPeerId=t.value;historyCompare=Boolean(t.value);render();return;}
 if(t.dataset.clipFilter){clipFilters[t.dataset.clipFilter]=t.type==='checkbox'?t.checked:t.value;refreshClipRail();focusPrecisionSelect($(`[data-clip-filter="${t.dataset.clipFilter}"]`));return;}
 if(t.hasAttribute('data-microphone-source')){if(busy||activeSession?.launch_locked||active())return;const source=t.value;t.value=replayPreferences?.microphone_mode==='selected'?replayPreferences.microphone_source:'default';enhancePrecisionSelects(workspace);perform(async()=>{replayPreferences=await call('set_replay_microphone',{mode:source==='default'?'default':'selected',source:source==='default'?null:source});render();notify('Microphone input saved for the next game launch.');});return;}
 if(t.id==='save-duration'){selectedSaveDuration=Number(t.value);updateObservedElements();return;}
 if(t.dataset.global){
  const key=t.dataset.global;
  if(key==='overlay'&&active()){
   const requested=t.checked;
   if(!native||!loaded.global||busy){t.checked=draft.overlay;return;}
   // Save just visibility. Pending appearance/recording edits retain their
   // own Save/Discard action and must not block a live show/hide request.
   draft.overlay=requested;updateHud();
   perform(async()=>{
    t.disabled=true;
    try{
     const saved=await call('save_global_settings',{input:profilePayload({...defaults,overlay:requested},savedShortcutMap),expectedRevision:globalRevision});
     defaults=mapDefaults(saved);globalRevision=saved.revision;
     draft.overlay=defaults.overlay;
     notify(saved.liveNotice||'Overlay visibility saved.');
    }catch(error){draft.overlay=defaults.overlay;throw error;}
    finally{t.disabled=false;t.checked=draft.overlay;updateHud();}
   });
   return;
  }
  if(key==='preset'){
   const next=t.value;
   if(next==='Custom' && draft.preset!=='Custom')draft.metrics=effectiveMetrics(draft.preset||'Compact',draft.metrics);
   draft.preset=next;
   syncMetricToggles();
  }else draft[key]=t.type==='checkbox'?t.checked:['fps','scale','opacity'].includes(key)?Number(t.value):t.value;
  updateHud();
  return;
 }
 if(t.dataset.replayPreference==='microphone'){
  const mode=t.checked?(replayPreferences?.microphone_source?'selected':'default'):'off';
  t.checked=replayPreferences?.microphone_mode!=='off';
  if(!native||!replayPreferences||busy||activeSession?.launch_locked||active())return;
  perform(async()=>{replayPreferences=await call('set_replay_microphone',{mode,source:replayPreferences.microphone_source||null});render();notify(mode==='off'?'Microphone capture disabled for new sessions.':'Microphone capture enabled for new sessions.');});
  return;
 }
 if(t.dataset.replayPreference==='outside'){
  const requested=t.checked;
  t.checked=replayPreferences?.close_overlay_on_outside_click===true;
  if(!native||!replayPreferences||busy)return;
  perform(async()=>{replayPreferences=await call('set_replay_overlay_behavior',{closeOnOutsideClick:requested});render();notify(requested?'Replay menu closes on outside click.':'Replay menu stays open until dismissed.');});
  return;
 }
 if(t.dataset.replayPreference==='initial-duration'){
  const requested=Number(t.value);
  if(!native||!replayPreferences||busy)return;
  perform(async()=>{replayPreferences=await call('set_replay_initial_save_duration',{durationSeconds:requested});render();notify('Replay menu initial duration saved.');});
  return;
 }
 if(t.dataset.preference==='tray'){
  const requested=t.checked;
  t.checked=preferences.tray===true;
  if(!native||!loaded.preferences||busy)return;
  perform(async()=>{
   updateObservedElements();
   const saved=await call('set_close_to_tray',{enabled:requested});
   preferences.tray=saved.close_to_tray;
   notify(preferences.tray?'Close to tray enabled.':'Close to tray disabled. Closing will quit Redunar.');
  });
  return;
 }
 if(t.dataset.preference==='diagnostic-log'){
  const requested=t.checked;
  t.checked=preferences.diagnosticLog===true;
  if(!native||!loaded.preferences||busy)return;
  perform(async()=>{
   const saved=await call('set_diagnostic_log',{enabled:requested});
   preferences.diagnosticLog=saved.diagnostic_log===true;
   notify(preferences.diagnosticLog?'Debug log enabled. Restart Redunar to start recording.':'Debug log disabled. Recording stops after the next restart.');
   render();
  });
  return;
 }
 if(t.dataset.preference==='automatic-updates'){
  const requested=t.checked;
  t.checked=preferences.automaticUpdates!==false;
  if(!native||!loaded.preferences||busy)return;
  perform(async()=>{
   updateObservedElements();
   const saved=await call('set_automatic_updates',{enabled:requested});
   preferences.automaticUpdates=saved.automatic_updates!==false;
   notify(preferences.automaticUpdates?'Automatic update checks enabled.':'Automatic update checks disabled.');
  });
  return;
 }
 if(t.dataset.override){
  if(t.value==='inherit')delete game().overrides[t.dataset.override];
  else game().overrides[t.dataset.override]=t.value==='On';
  render();focusPrecisionSelect($(`[data-override="${t.dataset.override}"]`));
 }
});
document.addEventListener('submit',event=>{
 if(event.target.id==='clip-filename-form'){event.preventDefault();const form=new FormData(event.target);const item=clips.find(c=>c.id===form.get('fileName'));const name=String(form.get('newFileName')).trim();const newFileName=item?renamedClipFilename(item.file_name,name):'';if(item&&name)perform(async()=>{const result=await call('rename_replay_clip',{fileName:item.file_name,newFileName});const oldId=item.id;item.id=result.file_name;item.file_name=result.file_name;if(selectedClip===oldId)selectedClip=item.id;refreshClipRail();if(selectedClip===item.id){const title=$('[data-selected-clip-title]');if(title)title.textContent=item.file_name;$('#clip-video')?.setAttribute('aria-label',item.file_name);}$('#dialog').close();notify('File renamed.');});return;}
 if(event.target.id==='trim-export-form'){event.preventDefault();const form=new FormData(event.target);runClipExport(String(form.get('fileName')),Number(form.get('start')),Number(form.get('end')),String(form.get('outputName')).trim(),replayPreferences?.resolved_directory);return;}
 if(event.target.id==='trash-clips-form'){event.preventDefault();const fileNames=JSON.parse(String(new FormData(event.target).get('fileNames')));perform(async()=>{await call('move_replay_clips_to_trash',{fileNames});await reloaders['reload-clips']();showClipTrashFeedback(fileNames);$('#dialog').close();render();});return;}
 if(event.target.id==='discovery-form'){
  event.preventDefault();if(busy)return;
  const form=new FormData(event.target),candidateIds=form.getAll('candidateId').map(String);
  if(!candidateIds.length){reportActionError('Select at least one game to import.');return;}
  const submit=event.target.querySelector('[type=submit]');
  submit.disabled=true;submit.textContent='Importing…';
  perform(async()=>{
   acceptGames(await call('import_discovered_games',{candidateIds}));
   $('#dialog').close();selectedGame=games.at(-1)?.id||selectedGame;render();
   const count=candidateIds.length;notify(`${count} game${count===1?'':'s'} imported into the local library.`);
  }).finally(()=>{submit.disabled=false;submit.textContent='Import selected';});
  return;
 }
 if(event.target.id==='edit-launch-form'){event.preventDefault();const form=new FormData(event.target);const args=String(form.get('arguments')||'').split(/\r?\n/).map(v=>v.trim()).filter(Boolean);const workingDirectory=String(form.get('workingDirectory')||'').trim()||null;perform(async()=>{acceptGames(await call('update_game_launch',{gameId:String(form.get('gameId')),executable:String(form.get('executable')).trim(),arguments:args,workingDirectory,expectedRevision:String(form.get('expectedRevision'))}));$('#dialog').close();render();notify('Launch settings saved.');});return;}
 if(event.target.id==='remove-game-form'){event.preventDefault();const gameId=String(new FormData(event.target).get('gameId'));perform(async()=>{acceptGames(await call('remove_game',{gameId}));selectedGame=games[0]?.id||null;$('#dialog').close();render();notify('Game removed from the local catalog.');});return;}
 if(event.target.id==='replay-folder-form'){event.preventDefault();const parent=String(new FormData(event.target).get('parent')).trim();perform(async()=>{replayPreferences=await call('set_replay_save_parent',{parent});clearClipTrashFeedback();await reloaders['reload-clips']();$('#dialog').close();render();notify('Replay folder saved.');});return;}
 if(event.target.id!=='add-game-form')return;event.preventDefault();
 const form=new FormData(event.target),name=String(form.get('name')).trim(),executable=String(form.get('exe')).trim();
 perform(async()=>{acceptGames(await call('add_game',{name,executable}));selectedGame=games.find(g=>g.name===name)?.id??selectedGame;$('#dialog').close();render();notify('Game added to the local library.');});
});
$('#dialog').addEventListener('cancel',event=>{if(exporting)event.preventDefault();});
window.addEventListener('native-error',event=>{
 if(pendingUpdateAction==='restarting'){
  pendingUpdateAction=null;updateStatus={...updateStatus,state:'error',message:String(event.detail)};
  updateObservedElements();
 }
 notify(event.detail);
});
window.addEventListener('focus',()=>{if(native&&route()==='library'&&loaded.catalog){installation.refresh();refreshSteamSetup($('.steam-setup-card'),game()?.id,call);}});
window.addEventListener('hashchange',()=>{const focusUpdates=focusUpdatesOnRender;render();if(!focusUpdates){workspace.focus({preventScroll:true});scrollRoot.scrollTo(0,0);}});

render();
    if(native){
 startRuntime({initialize:()=>Promise.all(Object.entries(reloaders).filter(([name])=>name!=='reload-diagnostics').map(([,reload])=>reload())),refresh:refreshRuntime,render,checkUpdates:checkUpdatesOnStartup,startPolling:poll,onError:error=>notify(message(error))});
}else{
 const connectionStatus=$('#connection-status');
 if(connectionStatus)connectionStatus.textContent='Browser inspection · backend disconnected';
 for(const section of Object.keys(loaded))errors[section]='Open the native app to load your saved data.';
 render();
}

// Scrolling never rebuilds the player or rail; only newly visible previews are queued.
document.addEventListener('scroll',event=>{if(event.target instanceof Element&&event.target.matches('.clip-list'))scheduleClipThumbnails();},true);
