import test from 'node:test';
import assert from 'node:assert/strict';
import { updateCard, updateSidebar } from '../ui/update-card.mjs';
const ready={native:true,busy:false,restartBlocked:false,pending:null};
test('A retryable installer failure retains its reason even with a cached update available',()=>{
 const html=updateCard({state:'available',operation_issue:true,message:'Authorization was cancelled.'},ready);
 assert.match(html,/Authorization was cancelled/);
 assert.doesNotMatch(html,/class="update-status-description" hidden/);
 assert.match(html,/data-action="install-update"/);
});
test('Restart is reserved for confirmed installation and blocked during a session',()=>{
 const status={state:'restart-needed',latest_version:'0.1.24'};
 assert.match(updateCard(status,ready),/data-action="restart-redunar" >Restart/);
 assert.match(updateCard(status,{...ready,restartBlocked:true}),/data-action="restart-redunar" disabled/);
 assert.doesNotMatch(updateCard({state:'handoff'},ready),/data-action="restart-redunar"/);
 assert.match(updateCard({state:'handoff'},ready),/Installation not confirmed/);
});
test('Pending operations disable actions without fabricating completion or progress',()=>{
 for(const pending of ['checking','installing','restarting']){
  const html=updateCard({state:'available'}, {...ready,pending});
  assert.match(html,/aria-busy="true"/);
  assert.doesNotMatch(html,/Update installed|\d+%/);
  assert.ok([...html.matchAll(/<button[^>]*>/g)].every(([tag])=>tag.includes('disabled')));
 }
});
test('Unavailable and failed updates expose escaped reasons and a retry action',()=>{
 for(const state of ['error','not-configured','handoff']){
  const html=updateCard({state,message:'Source <unavailable>'},ready);
  assert.match(html,/Source &lt;unavailable&gt;/);
  assert.match(html,/data-action="check-for-updates"/);
  assert.doesNotMatch(html,/Update installed/);
 }
});
test('No native connection or an ongoing action disables update controls',()=>{
 for(const options of [{...ready,native:false},{...ready,busy:true}]){
  const html=updateCard({state:'handoff',latest_version:'<next>'},options);
  assert.match(html,/v&lt;next&gt;/);
  assert.ok([...html.matchAll(/<button[^>]*>/g)].every(([tag])=>tag.includes('disabled')));
 }
});

test('Sidebar reports the running version with passive available or restart badges',()=>{
 for(const [state,label] of [['available','Update available'],['restart-needed','Restart needed']]){
  const html=updateSidebar({state,latest_version:'0.1.24'},{version:'0.1.23'});
  assert.match(html,/>v0.1.23<\/span>/);assert.doesNotMatch(html,/>v0.1.24<\/span>/);
  assert.ok(html.includes(label));assert.doesNotMatch(html,/<button|<a |data-action=/);
 }
});
test('Routine sidebar states show text only, with progress reserved for update operations',()=>{
 for(const state of ['unchecked','up-to-date','error','handoff','not-configured']){
  assert.doesNotMatch(updateSidebar({state},{version:'0.1.23'}),/<svg|sidebar-update-badge/);
 }
 for(const pending of ['installing','restarting']){
  assert.match(updateSidebar({state:'available'},{version:'0.1.23',pending}),/updater-spinner/);
 }
});
