import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

// Exercise the production dialog without starting the native app or scanning disk.
const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
const dialogSource=source.slice(source.indexOf('function discoveryModal('),source.indexOf('function stopPlayback('));
function renderDiscovery(games,candidates){
 let html;
 runInNewContext(`${dialogSource}\ndiscoveryModal(candidates);`,{
  games,candidates,escape:String,modal:(_title,body)=>{html=body;},
 });
 return html;
}
function candidate(source,source_id,launch_executable){
 return {candidate_id:'candidate',name:'Discovered game',source,source_id,launch_executable,install_directory:'/games/example',importable:Boolean(launch_executable)};
}
function assertSelectable(html,selectable){
 const checkbox=html.match(/<input[^>]*name="candidateId"[^>]*>/)?.[0];
 assert.ok(checkbox);
 assert.equal(/\bchecked\b/.test(checkbox),selectable);
 assert.equal(/\bdisabled\b/.test(checkbox),!selectable);
 const submit=html.match(/<button[^>]*type="submit"[^>]*>/)?.[0];
 assert.ok(submit);
 assert.equal(/\bdisabled\b/.test(submit),!selectable);
}

test('unimported Steam games remain selectable when another game shares their launcher',()=>{
 for(const executable of ['/usr/bin/steam','/usr/bin/flatpak']){
  const games=[{steam_app_id:949230,executable}]; // Cities: Skylines II
  for(const appId of [645630,960090,1273400]){
   const html=renderDiscovery(games,[candidate('Steam',appId,executable)]);
   assertSelectable(html,true);
   assert.doesNotMatch(html,/Already in Redunar library/);
  }
 }
});

test('an imported Steam app stays disabled even when its launcher changes or is unavailable',()=>{
 const games=[{steam_app_id:949230,executable:'/usr/bin/steam'}];
 for(const executable of ['/usr/bin/steam','/usr/bin/flatpak',null]){
  const html=renderDiscovery(games,[candidate('Steam',949230,executable)]);
  assertSelectable(html,false);
  assert.match(html,/Already in Redunar library/);
 }
});

test('non-Steam discovery still matches imported executables',()=>{
 const games=[{steam_app_id:null,executable:'/usr/bin/prismlauncher'}];
 const existing=renderDiscovery(games,[candidate('Desktop entry',null,'/usr/bin/prismlauncher')]);
 assertSelectable(existing,false);
 assert.match(existing,/Already in Redunar library/);
 assertSelectable(renderDiscovery(games,[candidate('Desktop entry',null,'/usr/bin/other-game')]),true);
});
