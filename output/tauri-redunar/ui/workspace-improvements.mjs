import { replayStatusCopy } from './replay-menu-view.mjs';

const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export const clipTitle = clip => clip?.file_name || 'Unnamed recording';
export function clipFilenameStem(filename) {
  const dot=filename.lastIndexOf('.');return dot>0?filename.slice(0,dot):filename;
}
export function renamedClipFilename(filename, name) {
  const dot=filename.lastIndexOf('.');return name.trim()+(dot>0?filename.slice(dot):'');
}
export function captureSummary(runtime) {
  const copy = replayStatusCopy(runtime);
  const ready=copy.statusClass==='ready';
  const recording = copy.statusClass!=='error'&&['Buffering','Saving'].includes(runtime?.phase);
  const shapes={ready:'<circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16 9"/>',pending:'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',error:'<path d="m12 3 10 18H2Z"/><path d="M12 9v5m0 3v.01"/>'};
  const buffer=Number(runtime?.buffered_seconds);
  const meta=ready?`${Number.isFinite(buffer)&&buffer>=0?`<span>${buffer.toFixed(0)} seconds buffered</span>`:''}${recording?`<span>${runtime.audio_active?'System audio active':'Waiting for system audio'}</span>`:''}`:'';
  return `<div class="capture-summary" data-state="${copy.statusClass}" ${copy.statusClass==='error'?'role="alert"':''}><div class="capture-status-symbol" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${shapes[copy.statusClass]}</svg></div><div class="capture-status-content"><strong class="capture-status-heading">${escape(copy.title)}</strong>${meta?`<div class="capture-status-meta">${meta}</div>`:''}${!ready&&copy.detail?`<p class="capture-status-detail">${escape(copy.detail)}</p>`:''}</div></div>`;
}
const validBytes = value => Number.isFinite(value) && value >= 0;
const bytes = value => !validBytes(value) ? '—' : value >= 1e9 ? `${(value/1e9).toFixed(1)} GB` : value >= 1e6 ? `${(value/1e6).toFixed(1)} MB` : value >= 1e3 ? `${(value/1e3).toFixed(1)} KB` : `${Math.round(value)} B`;
export function storageSummary(status) {
  const available = status?.available_bytes;
  const total = status?.total_bytes;
  const capacityKnown = validBytes(available) && validBytes(total) && total > 0 && available <= total;
  const usedPercent = capacityKnown ? 100 * (total - available) / total : null;
  const percentageLabel = capacityKnown ? usedPercent.toFixed(1).replace(/\.0$/, '') : null;
  const low = validBytes(available) && available < 5e9;
  const limit = status?.storage_limit_bytes;
  const limited = validBytes(limit) && limit < Number.MAX_SAFE_INTEGER;
  const over = limited && validBytes(status?.used_bytes) ? Math.max(0, status.used_bytes - limit) : 0;
  return `<section class="clip-storage${low?' low-space':''}" aria-label="Replay storage"><span class="storage-eyebrow">Replay drive</span><strong class="storage-free">${validBytes(available)?`${bytes(available)} <small>free</small>`:'Free space unavailable'}</strong>${capacityKnown?`<div class="drive-capacity" role="img" aria-label="${percentageLabel} percent of replay drive used"><i style="width:${usedPercent.toFixed(2)}%"></i></div><div class="drive-scale"><span>${percentageLabel}% drive used</span><span>${bytes(total)} total</span></div>`:''}<div class="storage-recordings"><span>Recordings</span><strong>${bytes(status?.used_bytes)}</strong></div>${limited?`<p class="storage-limit">${bytes(limit)} recording limit${over?` · ${bytes(over)} over limit`:''}</p>`:''}${low?'<p class="storage-warning" role="status">Low disk space. Open the clip folder to review recordings.</p>':''}</section>`;
}
export function clipFilterChips(filters) {
  const dateLabels = {'1':'Today','7':'Past week','30':'Past month'};
  return [['game',filters.game==='all'?null:filters.game],['date',filters.date==='all'?null:dateLabels[filters.date]]].filter(([,label])=>label).map(([key,label])=>`<button type="button" class="clip-filter-chip" data-remove-clip-filter="${key}" aria-label="Remove ${key} filter: ${escape(label)}">${escape(label)} <span aria-hidden="true">×</span></button>`).join('');
}
export function clipFiltersMarkup(clips, filters) {
  const games=[...new Set(clips.map(c=>c.game_name).filter(Boolean))].sort();
  const options=(entries,value)=>entries.map(([key,label])=>`<option value="${escape(key)}" ${value===key?'selected':''}>${escape(label)}</option>`).join('');
  const active=Number(filters.game!=='all')+Number(filters.date!=='all');
  const count=filterClips(clips,filters).length;
  return `<div class="clip-collection" role="group" aria-label="Clip collection"><button type="button" data-clip-scope="all" aria-pressed="${!filters.favorites}">All clips</button><button type="button" data-clip-scope="favorites" aria-pressed="${filters.favorites}"><span aria-hidden="true">★</span> Favorites</button></div><details class="clip-filter-panel" ${active?'open':''}><summary>Filters <span data-filter-count ${active?'':'hidden'}>${active}</span><span class="filter-chevron" aria-hidden="true">⌄</span></summary><div class="clip-filter-fields"><label>Game<select aria-label="Filter clips by game" data-clip-filter="game">${options([['all','All games'],...games.map(g=>[g,g])],filters.game)}</select></label><label>Date<select aria-label="Filter clips by date" data-clip-filter="date">${options([['all','Any date'],['1','Today'],['7','Past week'],['30','Past month']],filters.date)}</select></label></div></details><div class="clip-active-filters" aria-label="Active clip filters">${clipFilterChips(filters)}</div><div class="clip-results-toolbar"><span id="clip-result-count" aria-live="polite">${count} ${count===1?'clip':'clips'}</span><label class="clip-sort"><select aria-label="Sort clips" data-clip-filter="sort">${options([['newest','Newest first'],['oldest','Oldest first'],['title','Filename A–Z']],filters.sort)}</select></label></div>`;
}
export function filterClips(clips, filters, now = Date.now()) {
  const modified=c=>Number(BigInt(c.modified_unix_ns)/1000000n);
  const earliest=filters.date==='1'?new Date(new Date(now).setHours(0,0,0,0)).getTime():now-Number(filters.date)*86400000;
  return clips.filter(c=>(filters.game==='all'||c.game_name===filters.game)&&(!filters.favorites||c.favorite)&&(filters.date==='all'||modified(c)>=earliest)&&`${clipTitle(c)} ${c.file_name} ${c.game_name||''}`.toLowerCase().includes(filters.search.toLowerCase())).sort((a,b)=>filters.sort==='title'?clipTitle(a).localeCompare(clipTitle(b)):filters.sort==='oldest'?modified(a)-modified(b):modified(b)-modified(a));
}
export function comparisonSelector(sessions, selectedIndex, peerId) {
  const selected=sessions[selectedIndex];
  const peers=sessions.filter((s,index)=>index!==selectedIndex&&s.game===selected?.game);
  if(!peers.length)return '';
  return `<label class="comparison-selector">Compare with<select id="comparison-session" aria-label="Compare with session"><option value="">Choose a session</option>${peers.map(s=>`<option value="${escape(s.id)}" ${String(s.id)===peerId?'selected':''}>${escape(s.date)} · ${escape(s.time)} · ${escape(s.duration)}</option>`).join('')}</select></label>`;
}
export function microphoneSelector(devices, preferences, locked, error = null) {
  const selected=preferences?.microphone_mode==='selected'?preferences.microphone_source:'default';
  const off=preferences?.microphone_mode==='off';
  const help=locked?'End the game before changing the input.':off?'Turn on Microphone above to choose an input.':'Choose the input mixed with system audio. Applies to the next game launch.';
  return `<div class="field microphone-device"><div><label>Microphone input</label><p>${help}</p></div><div class="microphone-device-controls"><button class="button" type="button" data-action="reload-microphone-devices">Refresh inputs</button><select aria-label="Microphone input" data-microphone-source ${locked||off?'disabled':''}><option value="default" ${selected==='default'?'selected':''}>System default</option>${selected!=='default'&&!devices.some(d=>d.id===selected)?`<option value="${escape(selected)}" selected>Saved input unavailable</option>`:''}${devices.map(d=>`<option value="${escape(d.id)}" ${selected===d.id?'selected':''}>${escape(d.label)}</option>`).join('')}</select></div>${error?`<p class="small-note microphone-device-status" role="status">${escape(error)}</p>`:!devices.length?'<p class="small-note microphone-device-status">No microphone inputs found.</p>':''}</div>`;
}
