"""Refined production controls, with private native-command responses only."""
from webview_fixture import *

extra = r"""
const originalInvoke=window.__TAURI_INTERNALS__.invoke;
window.__TAURI_INTERNALS__.invoke=async(command,args={})=>{
 if(command==='trashed_replay_clips')return structuredClone(bridge.bin||[]);
 if(command==='move_replay_clips_to_trash'||command==='restore_replay_clips'){
  bridge.calls.push({command,args:structuredClone(args)});
  await new Promise(r=>setTimeout(r,80));
  if(bridge.failBin)throw Error('Fixture bin write failed');
  bridge.bin??=[];
  const trash=command==='move_replay_clips_to_trash',from=trash?bridge.clips:bridge.bin,to=trash?bridge.bin:bridge.clips;
  for(const name of args.fileNames){const i=from.findIndex(c=>c.file_name===name);if(i>=0)to.push(...from.splice(i,1));}
  return;
 }
 if(command==='set_beta_access'){
  bridge.calls.push({command,args:structuredClone(args)});
  if(bridge.failBeta)throw Error('Fixture preference write failed');
  bridge.beta=args.enabled;return {beta_access:bridge.beta};
 }
 if(command==='restart_redunar'){
  bridge.calls.push({command,args});if(bridge.acceptRestart)return;throw Error('Fixture restart blocked');
 }
 const result=await originalInvoke(command,args);
 if(command==='app_preferences')result.beta_access=Boolean(bridge.beta);
 if(command==='replay_runtime_status')result.microphone_state=bridge.micState||'off';
 return result;
};
"""
manager.add_script(WebKit2.UserScript.new(extra,WebKit2.UserContentInjectedFrames.TOP_FRAME,WebKit2.UserScriptInjectionTime.START,None,None))
try:
    view.load_uri(f'http://127.0.0.1:{server.server_port}/#replay');pump(1800)
    test('No clip multiselection is shipped',"check(!q('[data-select-clips]')&&!q('[data-clip-selection]'),'single clip actions only');check(q('[data-clip-filter=sort]'),'sort present');")
    js("click('[data-action=delete-selected-clip]')")
    test('Trash confirmation uses approved copy and cancel works',"check(q('#dialog-body').textContent.includes('Trashed clips can be restored from the trash bin.'),'restore copy');click('#dialog-body [data-close]');check(!q('#dialog').open,'cancel closes');click('[data-action=delete-selected-clip]');bridge.failBin=true;q('#trash-clips-form').requestSubmit();",200)
    test('Failed trash keeps the recording and permits retry',"check(q('#dialog').open,'dialog retained');check(q('#dialog-error').textContent.includes('Fixture bin write failed'),'write failure visible');check(qa('[data-clip]').length===3,'recordings retained');bridge.failBin=false;q('#trash-clips-form').requestSubmit();",300)
    test('Trash hides one clip and shows Undo',"check(qa('[data-clip]').length===2,'one hidden');check(!q('#clip-trash-feedback').hidden,'undo visible');check(bridge.bin.length===1,'one native bin entry');click('[data-action=undo-clip-trash]');",300)
    test('Undo restores the clip and clears the notice',"check(qa('[data-clip]').length===3,'restored');check(q('#clip-trash-feedback').hidden,'notice cleared');click('[data-action=delete-selected-clip]');q('#trash-clips-form').requestSubmit();",300)
    snap('refinements-trash-notice')
    pump(10300)
    test('Undo expires without removing bin contents',"check(q('#clip-trash-feedback').hidden,'ten second notice expires');check(bridge.bin.length===1,'restorable retained');click('[data-action=view-clip-trash]');",200)
    snap('refinements-trash-bin')
    test('Trash bin exposes restore without permanent deletion',"check(q('#dialog').open,'bin opens');check(q('[data-action=restore-trash-clip]'),'restore action');check(!q('[data-action=empty-trash]'),'no irreversible action');click('[data-action=restore-trash-clip]');",300)
    test('Restore returns the recording to Saved clips',"check(qa('[data-clip]').length===3,'all restored');check(!q('#dialog').open,'completed bin action closes');")
    route('history')
    test('History filters clear stale selected details',"q('#history-search').value='no matching session';q('#history-search').dispatchEvent(new Event('input',{bubbles:true}));check(!q('.history-detail'),'stale details removed');check(q('.history-filter-empty'),'empty state');click('[data-action=clear-history-filters]');check(q('.history-detail'),'selection restored');")
    route('global');js("click('[data-global-tab=overlay]');q('[data-global=scale]').value='105';q('[data-global=scale]').dispatchEvent(new Event('input',{bubbles:true}));")
    test('Draft marks both the settings tab and navigation',"check(!q('[data-global-tab=overlay] .unsaved-tab-marker').hidden,'tab dot');check(!q('a[href=\"#global\"] .nav-unsaved-marker').hidden,'sidebar dot');")
    route('library');route('global')
    test('Global draft survives app navigation',"check(q('[data-global=scale]').value==='105','draft retained');check(!q('.settings-save-bar').hidden,'save action retained');click('[data-action=discard-global]');")
    route('settings')
    test('Feature access defaults to Stable using existing preference',"check(q('#feature-access').selectedOptions[0].textContent==='Stable','stable default');q('#feature-access').value='beta';q('#feature-access').dispatchEvent(new Event('change',{bubbles:true}));",250)
    test('Beta selection writes beta access, never an update channel',"check(bridge.calls.findLast(c=>c.command==='set_beta_access').args.enabled,'existing preference');check(q('#feature-access').value==='beta','saved beta');bridge.failBeta=true;q('#feature-access').value='standard';q('#feature-access').dispatchEvent(new Event('change',{bubbles:true}));",250)
    test('Failed feature save retains saved Beta state',"check(q('#feature-access').value==='beta','failed write retained');check(q('#toast').textContent.includes('Fixture preference write failed'),'failure visible');bridge.failBeta=false;bridge.installedUpdate=true;click('[data-action=check-for-updates]');",200)
    test('Sidebar badge sits before readable version text',"const badge=q('.sidebar-update-badge'),version=q('.sidebar-version-number');check(badge&&version,'version state present');check(badge.getBoundingClientRect().right<version.getBoundingClientRect().left,'badge left with gap');check(version.closest('a').getAttribute('href')==='#settings','shared settings navigation');bridge.idle=true;window.dispatchEvent(new Event('focus'));",300)
    wait_for("!q('[data-action=restart-redunar]').disabled")
    test('Restart rejection clears pending state and remains recoverable',"click('[data-action=restart-redunar]');",300)
    test('Restart failure appears in Settings',"check(q('[data-update-state]').textContent==='Update error','native failure visible');check(q('[data-action=check-for-updates]')&&!q('[data-action=check-for-updates]').disabled,'recovery available');check(q('.update-status-description').textContent.includes('Fixture restart blocked'),'reason visible');")
    snap('refinements-updater-error')
    js("click('[data-action=check-for-updates]')");pump(250)
    route('global');js("click('[data-global-tab=overlay]');q('[data-global=scale]').value='110';q('[data-global=scale]').dispatchEvent(new Event('input',{bubbles:true}));")
    route('settings');js("click('[data-action=restart-redunar]')")
    test('Restart asks about retained drafts',"check(q('[data-action=save-and-restart]')&&q('[data-action=discard-and-restart]'),'explicit draft choices');click('#dialog-body [data-close]');")
    route('global')
    test('Cancelling restart preserves the draft',"check(q('[data-global=scale]').value==='110','cancel retained edits');bridge.conflict=true;")
    route('settings');js("click('[data-action=restart-redunar]');click('[data-action=save-and-restart]')");pump(300)
    test('Failed settings save blocks restart and preserves edits',"check(q('#dialog').open,'dialog retained');check(q('#dialog-error').textContent.includes('changed elsewhere'),'save failure');check(q('.nav-unsaved-marker').hidden===false,'draft retained');bridge.conflict=false;bridge.acceptRestart=true;click('[data-action=discard-and-restart]');",300)
    test('Native cleanup veto recovers the restarting controls',"check(q('[data-update-state]').textContent==='Restarting Redunar…','pending restart');window.dispatchEvent(new CustomEvent('native-error',{detail:'Fixture cleanup requires attention'}));check(q('[data-update-state]').textContent==='Update error','cleanup veto visible');check(!q('[data-action=check-for-updates]').disabled,'recovery enabled');")
    route('overview');js("bridge.idle=false;bridge.micState='active';window.dispatchEvent(new Event('focus'))");pump(1200)
    wait_for("q('[data-microphone-status]')?.textContent==='Microphone capturing'")
    test('Audio status comes from live runtime packets',"check(q('[data-microphone-status]').textContent==='Microphone capturing','active packets');check(q('[data-microphone-status]').dataset.audioState==='active','green active');bridge.micState='unavailable';window.dispatchEvent(new Event('focus'));",1200)
    wait_for("q('[data-microphone-status]')?.dataset.audioState==='unavailable'")
    test('Stalled microphone is unavailable',"check(q('[data-microphone-status]').dataset.audioState==='unavailable','red unavailable');")
    snap('refinements-overview-audio')
    route('replay')
    test('Replay shares the same microphone state',"check(q('[data-microphone-status]').textContent==='Microphone unavailable','shared native status');check(bridge.errors.length===0,bridge.errors.join('; '));")
    (ARTIFACTS/'refinements-checks.json').write_text(json.dumps(checks,indent=2)+'\n')
    print(f'PASS: {len(checks)} refinements checks. Screenshots: {ARTIFACTS}')
finally:
    window.destroy();server.shutdown()
