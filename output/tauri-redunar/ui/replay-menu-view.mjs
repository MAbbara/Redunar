import { measurement } from './native-data.mjs';
const escape = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

export function replayStatusCopy(value) {
 const phase=String(value?.phase||'Unavailable');
 if(value?.failure)return {title:'Replay unavailable',detail:value.failure,statusClass:'error'};
 if(value?.unavailable_reason)return {title:'Replay unavailable',detail:value.unavailable_reason,statusClass:'error'};
 if(phase==='Inactive'&&value?.pending_reason)return {title:'Waiting for Replay frames',detail:value.pending_reason,statusClass:'pending'};
 if(value?.can_save)return {title:'Ready to save',detail:`${measurement(value.buffered_seconds)}s buffered`,statusClass:'ready'};
 if(phase==='Buffering')return {title:'Buffering replay',detail:`${measurement(value?.buffered_seconds)}s buffered · building the local replay buffer`,statusClass:'pending'};
 if(phase==='Saving')return {title:'Saving replay',detail:'Writing the selected moment to local storage…',statusClass:'pending'};
 if(phase==='Inactive')return {title:'Replay inactive',detail:'Launch a supported game to build a local buffer.',statusClass:'pending'};
 if(phase==='Unavailable')return {title:'Replay unavailable',detail:value?.unavailable_reason||'Launch a supported game to start Replay.',statusClass:'error'};
 return {title:'Replay unavailable',detail:'The local replay runtime is not ready yet.',statusClass:'error'};
}
