import test from 'node:test';
import assert from 'node:assert/strict';
import { filterClips, clipTitle, comparisonSelector, captureSummary, storageSummary, microphoneSelector, clipFiltersMarkup, clipFilterChips, clipFilenameStem, renamedClipFilename } from '../ui/workspace-improvements.mjs';
const now=new Date('2026-10-14T12:00:00Z').getTime();
const record=(id,age,game_name,extra={})=>({file_name:id,title:id,game_name,modified_unix_ns:String(BigInt(now-age*86400000)*1000000n),...extra});
const clips=[record('a',0,'Circuit',{display_title:'Zoom',favorite:true}),record('b',3,'Drive'),record('c',40,'Circuit',{display_title:'Apex'})];
const filters={search:'',game:'all',date:'all',sort:'newest',favorites:false};
test('Filters compose game, date, search and favorite without mutating library',()=>{const snapshot=structuredClone(clips);assert.deepEqual(filterClips(clips,{...filters,game:'Circuit',date:'7',search:'a',favorites:true},now).map(c=>c.file_name),['a']);assert.deepEqual(clips,snapshot);assert.equal(clipTitle(clips[0]),'a');});
test('Filename and chronological sorting retain filename identities',()=>{assert.deepEqual(filterClips(clips,{...filters,sort:'oldest'},now).map(c=>c.file_name),['c','b','a']);assert.deepEqual(filterClips(clips,{...filters,sort:'title'},now).map(c=>c.file_name),['a','b','c']);});
test('Date filters exclude older clips while unfiltered includes every age',()=>{assert.equal(filterClips(clips,{...filters,date:'7'},now).length,2);assert.equal(filterClips(clips,filters,now).length,3);assert.equal(filterClips(clips,{...filters,search:'missing'},now).length,0);});
test('Comparison allows only another same-game session and escapes labels',()=>{const s=[{id:1,game:'a'},{id:2,game:'a',date:'<today>',time:'12:00',duration:'30m'},{id:3,game:'b'}];const html=comparisonSelector(s,0,'2');assert.match(html,/value="2" selected/);assert.doesNotMatch(html,/value="1"|value="3"/);assert.match(html,/&lt;today&gt;/);});
test('Capture failure does not claim buffering or active audio',()=>{const html=captureSummary({phase:'Failed',can_save:false,failure:'Encoder failed'},{});assert.match(html,/data-state="error"/);assert.match(html,/Encoder failed/);assert.doesNotMatch(html,/seconds buffered|System audio active/);});
test('Shared capture summary groups actual buffer and audio status',()=>{const runtime={phase:'Buffering',can_save:true,buffered_seconds:32,audio_active:true};assert.match(captureSummary(runtime),/32 seconds buffered.*System audio active/);assert.doesNotMatch(captureSummary(runtime),/Open Replay|Set a save shortcut/);});
test('Disk space distinguishes low, available and unavailable without deletion policy',()=>{assert.match(storageSummary({available_bytes:1e9,total_bytes:10e9}),/low-space/);assert.doesNotMatch(storageSummary({available_bytes:20e9,total_bytes:100e9}),/low-space/);assert.match(storageSummary(null),/unavailable/);});
test('Microphone selection is disabled during a launch and retains selected identity',()=>{const html=microphoneSelector([{id:'usb',label:'USB <mic>'}],{microphone_mode:'selected',microphone_source:'usb'},true);assert.match(html,/data-microphone-source disabled/);assert.match(html,/value="usb" selected/);assert.match(html,/USB &lt;mic&gt;/);});
test('Explicit rejection wins over conflicting readiness fields',()=>{const html=captureSummary({phase:'Buffering',can_save:true,buffered_seconds:100,audio_active:true,unavailable_reason:'Capture source rejected'},{});assert.match(html,/data-state="error"/);assert.match(html,/Capture source rejected/);assert.doesNotMatch(html,/seconds buffered|System audio active/);});

test('Applied clip filters remain visible and removable with escaped game identity',()=>{
 const filtered={...filters,game:'<Circuit>',date:'7'};
 const chips=clipFilterChips(filtered);
 assert.match(chips,/&lt;Circuit&gt;/);assert.match(chips,/Remove game filter: &lt;Circuit&gt;/);assert.match(chips,/Past week/);
 const markup=clipFiltersMarkup(clips,filtered);
 assert.match(markup,/data-filter-count >2/);assert.match(markup,/data-clip-scope="all" aria-pressed="true"/);
 assert.match(clipFiltersMarkup(clips,{...filters,favorites:true}),/data-clip-scope="favorites" aria-pressed="true"/);
 assert.equal(clipFilterChips(filters),'');
});
test('Storage separates whole-drive usage from recording bytes',()=>{
 const html=storageSummary({available_bytes:40e9,total_bytes:100e9,used_bytes:18e9});
 assert.match(html,/40.0 GB <small>free/);assert.match(html,/60% drive used/);assert.match(html,/100.0 GB total/);assert.match(html,/Recordings.*18.0 GB/);
});
test('Unknown or invalid capacity preserves recording size without inventing a meter',()=>{
 for(const status of [{available_bytes:null,total_bytes:0},{available_bytes:-1,total_bytes:100},{available_bytes:200,total_bytes:100},{available_bytes:NaN,total_bytes:Infinity}]){
  const html=storageSummary({...status,used_bytes:29.8e6});assert.doesNotMatch(html,/drive-capacity|NaN|Infinity|width:/);assert.match(html,/29.8 MB/);
 }
});
test('Low disk and recording-budget warnings remain distinct and never delete recordings',()=>{
 const html=storageSummary({available_bytes:950e6,total_bytes:100e9,used_bytes:12e9,storage_limit_bytes:10e9});
 assert.match(html,/950.0 MB <small>free/);assert.match(html,/Low disk space/);assert.match(html,/10.0 GB recording limit.*2.0 GB over limit/);assert.doesNotMatch(html,/data-action="delete|automatic/);
});

test('Filename editing hides and preserves the original extension, including names with dots',()=>{
 assert.equal(clipFilenameStem('Lap.final.mp4'),'Lap.final');
 assert.equal(renamedClipFilename('Lap.final.mp4',' New lap '),'New lap.mp4');
 assert.equal(renamedClipFilename('Lap.final.mp4','Lap.webm'),'Lap.webm.mp4');
 assert.equal(renamedClipFilename('clip.MP4','Lap.final'),'Lap.final.MP4');
});
test('Both pages preserve escaped failures without readiness or audio claims',()=>{
 for(const page of ['overview','replay']) {
  const html=captureSummary({phase:'Buffering',can_save:true,buffered_seconds:120,audio_active:true,failure:'Encoder <failed>'},{30:'F8'},30,page);
  assert.match(html,/role="alert"/);assert.match(html,/Encoder &lt;failed&gt;/);assert.doesNotMatch(html,/Ready to save|seconds buffered|System audio active/);
  assert.doesNotMatch(html,/Open Replay/);
 }
});
test('Pending frames show the explanation and do not claim a ready buffer',()=>{
 const html=captureSummary({phase:'Inactive',can_save:false,pending_reason:'Waiting for game frames'}, {},30,'replay');
 assert.match(html,/data-state="pending"/);assert.match(html,/Waiting for game frames/);assert.doesNotMatch(html,/Ready to save|seconds buffered|role="alert"/);
});

test('Unavailable saved microphone remains selected without inventing a default',()=>{
 const html=microphoneSelector([],{microphone_mode:'selected',microphone_source:'usb<&'},false,'Audio server unavailable');
 assert.match(html,/value="usb&lt;&amp;" selected>Saved input unavailable/);assert.match(html,/Audio server unavailable/);assert.match(html,/Refresh inputs/);assert.doesNotMatch(html,/value="default" selected/);
});
