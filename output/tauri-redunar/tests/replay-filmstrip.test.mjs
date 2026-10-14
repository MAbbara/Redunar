import {test} from 'node:test';
import assert from 'node:assert/strict';
import {renderReplayFilmstrip} from '../ui/replay-filmstrip.mjs';

const tick=()=>new Promise(resolve=>setTimeout(resolve,40));
function fixture({playing=false,hidden=false,fail=false}={}){
 const draws=[];
 class Preview extends EventTarget {
  videoWidth=640;videoHeight=360;position=0;sources=[];released=false;style={};attached=false;
  setAttribute(){}
  remove(){this.attached=false;}
  set src(value){this.sources.push(value);queueMicrotask(()=>this.dispatchEvent(new Event(fail?'error':'loadeddata')));}
  get currentTime(){return this.position;}
  set currentTime(value){this.position=value;queueMicrotask(()=>this.dispatchEvent(new Event('seeked')));}
  removeAttribute(name){if(name==='src')this.released=true;}
  load(){}
 }
 const preview=new Preview();
 const document=Object.assign(new EventTarget(),{hidden,createElement:()=>preview,body:{append:video=>{video.attached=true;}}});
 globalThis.document=document;
 const player=Object.assign(new EventTarget(),{paused:!playing,currentSrc:'http://local/selected-token',duration:30,currentTime:9});
 const canvas={dataset:{},getContext:()=>({drawImage:(video,...args)=>draws.push({time:video.currentTime,args})})};
 const status={hidden:false,textContent:'Loading frame previews…'};
 const stop=renderReplayFilmstrip(player,canvas,status);
 return {preview,player,canvas,status,draws,stop,document};
}

test('filmstrip samples eight moments within a fixed canvas without seeking the player',async()=>{
 const f=fixture();
 for(let i=0;i<20&&!f.status.hidden;i++)await tick();
 assert.equal(f.draws.length,8);
 assert.equal(f.draws[0].time,0);
 assert.equal(f.draws.at(-1).time,29.9);
 assert.equal(f.player.currentTime,9);
 assert.deepEqual(f.preview.sources,['http://local/selected-token']);
 assert.equal(f.canvas.width,1280);assert.equal(f.canvas.height,90);
 assert.equal(f.status.hidden,true);assert.equal(f.preview.released,true);assert.equal(f.preview.attached,false);
 f.stop();
});

test('selection cancellation prevents late samples and releases the decoder',async()=>{
 const f=fixture();
 await tick();f.stop();
 const completed=f.draws.length;
 await tick();await tick();
 assert.equal(f.draws.length,completed);
 assert.equal(f.preview.released,true);
});

test('sampling waits for playback and hidden windows without starting a stream',async()=>{
 const f=fixture({playing:true,hidden:true});
 await tick();assert.equal(f.preview.sources.length,0);
 f.player.paused=true;f.player.dispatchEvent(new Event('pause'));
 await tick();assert.equal(f.preview.sources.length,0);
 f.document.hidden=false;f.document.dispatchEvent(new Event('visibilitychange'));
 await tick();assert.equal(f.preview.sources.length,1);
 f.stop();
});

test('preview failure is contained and leaves the player untouched',async()=>{
 const f=fixture({fail:true});
 await tick();
 assert.equal(f.draws.length,0);assert.equal(f.preview.released,true);
 assert.match(f.status.textContent,/Frame previews unavailable/);
 assert.equal(f.player.currentTime,9);
 f.stop();
});
