// Pure presentation and filtering for the native workspace.
export const escapeHtml = value => String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function recordingAudio(status) {
 const microphone=status.microphone_state||'unknown';
 const micText={active:'Microphone capturing',off:'Microphone off',unavailable:'Microphone unavailable',waiting:'Waiting for microphone',unknown:'Microphone status unknown'}[microphone]||'Microphone status unknown';
 return `<span class="recording-audio ${status.audio_active?'':'audio-warning'}" data-audio-state="${status.audio_active?'active':'unavailable'}">${status.audio_active?'System audio capturing':'Waiting for system audio'}</span><span class="recording-audio ${microphone==='unavailable'?'audio-warning':''}" data-microphone-status data-audio-state="${microphone==='active'?'active':microphone==='unavailable'?'unavailable':'inactive'}">${micText}</span>`;
}
export function dirtyGlobalTabs(draft,saved,shortcutDirty) {
 const changed=key=>JSON.stringify(key==='metrics'?[...(draft[key]||[])].sort():draft[key])!==JSON.stringify(key==='metrics'?[...(saved[key]||[])].sort():saved[key]);
 return {overlay:['overlay','preset','layout','palette','branding','position','scale','opacity','metrics','captureMetrics'].some(changed),replay:['fps','quality','format'].some(changed),shortcuts:shortcutDirty};
}
const options=(values,current)=>values.map(([value,label])=>`<option value="${escapeHtml(value)}" ${String(current)===String(value)?'selected':''}>${escapeHtml(label)}</option>`).join('');
export function historyFiltersMarkup(sessions,filters) {
 const games=[...new Set(sessions.map(s=>s.game))].sort();
 return `<div class="history-filter-fields"><label>Game<select data-history-filter="game" aria-label="Filter history by game">${options([['all','All games'],...games.map(g=>[g,g])],filters.game)}</select></label><label>Date<select data-history-filter="date" aria-label="Filter history by date">${options([['all','Any date'],['1','Today'],['7','Past week'],['30','Past month']],filters.date)}</select></label></div>`;
}
export function filterSessions(sessions,filters,now=Date.now()) {
 const earliest=filters.date==='all'?0:filters.date==='1'?new Date(new Date(now).setHours(0,0,0,0)).getTime():now-Number(filters.date)*86400000;
 return sessions.map((session,index)=>({...session,index})).filter(s=>(filters.game==='all'||s.game===filters.game)&&s.started_unix*1000>=earliest&&s.game.toLowerCase().includes(filters.search.toLowerCase()));
}
export function clipSortingToolbar(total, sort) {
 const sortOptions=options([['newest','Newest first'],['oldest','Oldest first'],['title','Filename A–Z']],sort);
 return `<div class="clip-results-toolbar" role="group" aria-label="Clip sorting"><span id="clip-result-count" aria-live="polite">${total} ${total===1?'clip':'clips'}</span><div class="clip-sort"><select aria-label="Sort clips" data-clip-filter="sort">${sortOptions}</select></div></div>`;
}
