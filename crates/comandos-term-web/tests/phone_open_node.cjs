// A phone opening a session never sizes the shared tmux window: the first attach
// uses ignore-size, and only a one-pane listing takes the phone width (29-oct).
// node phone_open_node.cjs <nodejs build of comandos_term_web>/comandos_term_web.js
const assert=require('node:assert/strict'),path=require('node:path');
const tests=__dirname;
global.__terminalPageFixtureOnly=true;const f=require(path.join(tests,'terminal_controls.cjs'));delete global.__terminalPageFixtureOnly;
const {doc,makeEl}=f;
Object.assign(global,{HTMLElement:class{static[Symbol.hasInstance](o){return o?.nodeType===1}},HTMLTextAreaElement:class{static[Symbol.hasInstance](o){return o?.tagName==='TEXTAREA'}},HTMLCanvasElement:class{static[Symbol.hasInstance](o){return o?.tagName==='CANVAS'}},CanvasRenderingContext2D:class{static[Symbol.hasInstance](o){return typeof o?.measureText==='function'}},CanvasRenderingContext2d:class{static[Symbol.hasInstance](o){return typeof o?.measureText==='function'}},MessageEvent:class{static[Symbol.hasInstance](o){return o&&'data'in o}},devicePixelRatio:1,performance:{now:()=>0},FontFace:class{constructor(){this.loaded=Promise.resolve(this)}load(){return this.loaded}}});
global.innerWidth=390;global.innerHeight=800;
doc.fonts={add(){},delete(){},check:()=>true,load:()=>Promise.resolve([]),ready:Promise.resolve()};
const context={canvas:null,measureText:()=>({width:8,actualBoundingBoxAscent:11,actualBoundingBoxDescent:3,fontBoundingBoxAscent:11,fontBoundingBoxDescent:3}),setTransform(){},scale(){},fillRect(){},clearRect(){},beginPath(){},moveTo(){},lineTo(){},stroke(){},fillText(){},save(){},restore(){},rect(){},clip(){},getImageData:()=>({data:new Uint8ClampedArray(4)}),putImageData(){},createPattern:()=>({})};
const create=doc.createElement;doc.createElement=tag=>{const n=create(tag);n.removeChild=c=>{c.remove();return c};n.getClientRects=()=>[n.getBoundingClientRect()];n.getBoundingClientRect=()=>({left:0,top:0,width:tag==='span'?8:640,height:tag==='span'?14:240,right:640,bottom:240});n.getContext=()=>({...context,canvas:n});return n;};
global.getComputedStyle=()=>({getPropertyValue:()=>'',paddingLeft:'0px',paddingRight:'0px',paddingTop:'0px',paddingBottom:'0px',marginLeft:'0px',marginRight:'0px',width:'640px',height:'240px',fontSize:'14px',lineHeight:'16px'});
let panes=[];
const split=[{id:'%1',active:true,left:0,top:0,width:75,height:26},{id:'%2',active:false,left:76,top:0,width:76,height:26}];
global.__calls=[];global.fetch=async(p,o)=>(global.__calls.push(p+' '+(o&&o.body||'')),{ok:true,json:async()=>p==='/terminal-panes'?{ok:true,panes}:p.startsWith('/tab-models')?{panes:[]}:p.startsWith('/workspace/client')?{drafts:{},anchors:{}}:{}});
const head=makeEl('head',{},f.root);f.root.prepend(head);head.appendChild(makeEl('meta',{name:'comandos-web-mode',content:'native-term'},head));
const wasm=require(path.resolve(process.argv[2]));
const ticks=async()=>{for(let i=0;i<20;i++)await new Promise(r=>setImmediate(r));};
let intervals=[];const run=async()=>{const due=f.timers.splice(0);intervals.push(...due.filter(t=>t[1]>=1000));for(const [fn] of intervals){try{fn()}catch(e){}}await ticks();for(const [fn] of due){try{fn()}catch(e){}}await ticks();};
const open=async(i)=>{await run();const s=f.sockets[i];assert.ok(s,'socket '+i);s.readyState=1;s.protocol='comandos.term.v1';s.closes=[];s.close=(c,r)=>s.closes.push(r);s.fire('open');await ticks();return [s,JSON.parse(s.sent[0])];};
const reopen=async(s)=>{s.readyState=3;s.fire('close');await run();await run();return open(f.sockets.length-1);};
(async()=>{
  wasm.boot('native-term-private');await ticks();
  let [s,init]=await open(0);
  assert.equal(init.ignoreSize,true,'a phone opens without sizing the window');
  await run();
  assert.deepEqual(s.closes,[],'an empty listing changes nothing');
  panes=split;await run();await run();
  if(s.closes.length){[s,init]=await reopen(s);}
  assert.equal(init.ignoreSize,true,'a split: the grid takes the window size, still ignore-size');
  assert.equal(init.cols,152,'the cropped grid is the whole window');
  await run();
  panes=[split[0]];await run();await run();
  assert.ok(s.closes.includes('resize'),'one pane: reconnect');
  [s,init]=await reopen(s);
  assert.equal(init.ignoreSize,undefined,'one pane: the phone takes its own width');
  console.log('PASS phone opens without sizing the shared window; one pane takes the phone width');
})().catch(e=>{console.error(e);process.exitCode=1});
