// Runs the actual integrated WASM library against the compiled page body, offline.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const wasmPath=path.resolve(process.argv[2]||'');
const scenarios=['early','late','topological','unknown','attach','startup','ack','gradual','content-failed','content-missing','news-use','news-cancel','news-deeplink','news-desktop','app-bridge','sound-gesture','models-empty','models-null','models-false','http-native','sidebar-views'];
if(!process.argv[3]){for(const scenario of scenarios){const r=cp.spawnSync(process.execPath,[__filename,wasmPath,scenario],{encoding:'utf8'});process.stdout.write(r.stdout);process.stderr.write(r.stderr);assert.equal(r.status,0,scenario);}console.log('Full native boot: '+scenarios.length+' actual WASM DOM/startup/failure scenarios passed');process.exit(0);}
const scenario=process.argv[3],{doc,makeEl,parse}=require('./full_native_boot_dom.cjs');
const registry=fs.readFileSync(path.resolve(__dirname,'../src/registry.rs'),'utf8');
const inventory=JSON.parse(fs.readFileSync(path.resolve(__dirname,'../../../xtask/web/inventory.json')));const ids=inventory.pages['index.html'].map(id=>inventory.units.find(u=>u.id===id).component);assert.equal(ids.length,48);if(scenario==='topological'){const ordered=[],seen=new Set();function visit(id){if(seen.has(id))return;seen.add(id);const metadata=JSON.parse(fs.readFileSync(path.resolve(__dirname,'../components/'+id+'.json')));for(const dep of metadata.deps||[])if(ids.includes(dep))visit(dep);ordered.push(id);}for(const id of ids)visit(id);ids.splice(0,ids.length,...ordered);}for(const id of ids)assert(registry.includes('id: \"'+id+'\",'),'registered '+id);
const root=makeEl('html',{},null),head=makeEl('head',{},root);root.appendChild(head);
head.appendChild(makeEl('meta',{name:'comandos-web',content:scenario==='gradual'?'':ids.join(' ')+(scenario==='unknown'?' missing-component':'')},head));
if(scenario!=='gradual')head.appendChild(makeEl('meta',{name:'comandos-web-mode',content:'native'},head));
doc.documentElement=root;doc.head=head;doc.readyState=scenario==='early'||scenario==='gradual'?'loading':'interactive';doc.body=null;
function installBody(){doc.body=makeEl('body',{},root);root.appendChild(doc.body);parse(fs.readFileSync(path.resolve(__dirname,'../../comandos-web-view/src/index_page_body.html'),'utf8'),doc.body);const form=doc.body.querySelector('#srv-form');for(const k of['host','hostname','user','port','identity','orig','remember'])form[k]=form.querySelector('[name="'+k+'"]');form.reset=()=>{};}
if(scenario!=='early'&&scenario!=='gradual')installBody();
doc.querySelector=s=>root.querySelector(s);doc.querySelectorAll=s=>root.querySelectorAll(s);doc.getElementById=id=>root.querySelector('#'+id);doc.cookie='';doc.hidden=false;doc.visibilityState='visible';
const calls=[],timers=[],listeners={},storage=new Map(),errors=[],contentLoads=[];let finishConf,finishAck,finishContent;const bridgeMessages=[];
process.on('unhandledRejection',e=>errors.push(String(e)));
Object.defineProperty(global,'navigator',{configurable:true,writable:true,value:{}});
Object.assign(global,{window:global,document:doc,location:{protocol:'https:',hostname:'dash.private.invalid',origin:'https://dash.private.invalid',href:'https://dash.private.invalid/?web=native',search:'?web=native',reload(){throw Error('unexpected reload')}},navigator:{platform:'Linux',maxTouchPoints:0,language:'en',onLine:true,sendBeacon:()=>true},innerWidth:1100,innerHeight:800,devicePixelRatio:1,localStorage:{getItem:k=>storage.get(k)||null,setItem:(k,v)=>storage.set(k,String(v)),removeItem:k=>storage.delete(k)},sessionStorage:{getItem:()=>null,setItem(){}},matchMedia:()=>({matches:false,addEventListener(){},removeEventListener(){}}),getComputedStyle:()=>({getPropertyValue:()=>'',fontSize:'14px'}),addEventListener:(t,f)=>(listeners[t]||=[]).push(f),removeEventListener(){},dispatchEvent(){},setTimeout:(f,ms)=>{timers.push([f,ms]);return timers.length},clearTimeout(){},setInterval:(f,ms)=>{timers.push([f,ms]);return timers.length},clearInterval(){},requestAnimationFrame:f=>{timers.push([f,0]);return timers.length},cancelAnimationFrame(){},ResizeObserver:class{observe(){}disconnect(){}},MutationObserver:class{observe(){}disconnect(){}},Notification:class{static permission='denied'},CustomEvent:class{constructor(type,options){this.type=type;Object.assign(this,options)}},Event:class{constructor(type,options){this.type=type;Object.assign(this,options)}},Element:class{static[Symbol.hasInstance](o){return o?.nodeType===1}},Window:class{static[Symbol.hasInstance](o){return o===global}},HTMLElement:class{static[Symbol.hasInstance](o){return o?.nodeType===1}},HTMLInputElement:class{static[Symbol.hasInstance](o){return o?.tagName==='INPUT'}}});
if(scenario==='http-native')Object.assign(location,{protocol:'http:',hostname:'127.0.0.1',origin:'http://127.0.0.1:4777',href:'http://127.0.0.1:4777/?web=native'});
if(scenario==='news-desktop'){location.search='?panel=news';storage.set('cc_token','private-news-token');}
if(scenario==='news-deeplink')location.search='?news=2026-10-06@08:00';
if(scenario==='app-bridge'||scenario==='news-desktop')global.webkit={messageHandlers:{centro:{postMessage:m=>bridgeMessages.push(typeof m==='string'?JSON.parse(m):m)}}};
const oldCreate=doc.createElement;doc.createElement=tag=>{const n=oldCreate(tag);if(tag==='canvas')n.getContext=()=>({measureText:s=>({width:s.length*7}),fillRect(){},clearRect(){},fillText(){},beginPath(){},moveTo(){},lineTo(){},stroke(){},scale(){},setTransform(){}});return n;};
const routes={ '/conf':{_lang:'en',VOLUME:'50',SPEAK_DONE:'0',SPEAK_ATTENTION:'0'},'/prefs':{favorites:[],theme:'dia',button_style:'sutil',tabs_layout:'row'},'/remote-state':{remoteOn:false,terminalState:'off',urls:{},qrAvailable:false},'/ssh':[], '/tabs':[], '/state':[], '/tab-history':[], '/usage/state':{},'/chains':{chains:[]},'/commands/catalog':{catalog:{commands:[],groups:[]},cliInPane:'codex'},'/workspace':{revision:1,groups:[],tabs:[]},'/analytics/week':{},'/news/editions':{enabled:true,editions:[]}};
if(scenario.startsWith('models-')) routes['/models/latest']={ 'models-empty':{}, 'models-null':null, 'models-false':{newSince:false}}[scenario];
function response(payload,ok=true){return{ok,status:ok?200:400,statusText:ok?'OK':'Bad Request',json:async()=>structuredClone(payload),text:async()=>JSON.stringify(payload)}}
global.fetch=async(input,options={})=>{const url=typeof input==='string'?input:input.url;const p=new URL(url,location.origin).pathname;calls.push({path:p,query:new URL(url,location.origin).search,options});if(p==='/conf'&&!finishConf){for(const name of['api','tf','initApp','mountCommandSidebar','mountChainPage','loadRemote','loadPrefs','loadSsh','loadConf','arm','tick','rowKey','render','quickTerminalInstance','selectPaneInFrame'])assert.equal(typeof global[name],'function','global mounted before startup: '+name);assert(global.openTerms instanceof Map);assert(global.S);if(scenario==='startup'){return new Promise(r=>finishConf=()=>r(response(routes[p])));}}if(p==='/web/ready'){if(scenario==='ack')return response({},false);if(scenario==='early')return new Promise(r=>finishAck=()=>r(response({})));return response({});}if(p.startsWith('/workspace/client'))return response({activeTabId:null});if(p==='/presence')return response({});if(scenario==='news-desktop'&&p==='/news/editions')return response({configured:true,latest:'2026-10-06@08:00',editions:[{id:'2026-10-06@08:00',status:'published',storyCount:1}]});if(scenario==='news-desktop'&&p==='/news/edition')return response({id:'2026-10-06@08:00',status:'published',stories:[{id:'story-fixture',title:'Resumen protegido',summary:'**Contenido protegido**',sources:[]}],notes:[]});if(scenario==='news-desktop'&&p==='/web/markdown'){assert.equal(options.headers['X-Comandos-Token'],'private-news-token','reader Markdown uses the native dashboard credential');return response({html:'<p><strong>Contenido protegido</strong></p>'});}if(scenario==='news-desktop'&&(p==='/news/saved'||p==='/news/notes'))return response([]);if(p==='/news/edition')return response({id:'2026-10-06@08:00',stories:[],notes:[],status:'empty'});if(!(p in routes))throw Error('unexpected offline route '+p);return response(routes[p]);};
const wasm=require(wasmPath);
assert.equal(wasm.needs_content(),scenario!=='gradual');
if(scenario!=='gradual')require(process.env.COMANDOS_TEST_SOUND_MODULE).register_sound();
wasm.install_content_loader(()=>{
 contentLoads.push('import');
 if(scenario==='content-failed'||scenario==='content-missing')return Promise.reject(Error('fixture deferred content unavailable'));
 const complete=()=>{require(process.env.COMANDOS_TEST_CONTENT_MODULE).register_content();return undefined;};
 if(scenario==='news-cancel'||scenario==='analytics-close')return new Promise(resolve=>finishContent=()=>resolve(complete()));
 return Promise.resolve(complete());
});
if(scenario==='attach')doc.getElementById('poll').remove();
const settle=async()=>{for(let i=0;i<20;i++)await new Promise(r=>setImmediate(r));};
(async()=>{
 wasm.boot('native-private-fixture');
 if(scenario==='early'){assert.equal(global.S,undefined);assert.equal(calls.length,0);assert.equal(__comandosBootState,'waiting-dom');wasm.boot('ignored');assert.equal(doc.listenerCount('DOMContentLoaded'),1);installBody();doc.readyState='interactive';doc.dispatch('DOMContentLoaded');}
 if(scenario==='gradual'){assert.equal(__comandosReady,true);assert.equal(calls.filter(c=>c.path==='/web/ready').length,1);console.log('PASS gradual head compatibility');return;}
 await settle();
 if(scenario==='startup'){assert.equal(typeof finishConf,'function');doc.getElementById('tab-prev').remove();finishConf();await settle();}
 if(scenario==='early'){assert.equal(typeof finishAck,'function',JSON.stringify(global.__comandosBootReport));assert.notEqual(global.__comandosReady,true);assert.equal(__comandosBootState,'reporting');finishAck();await settle();}
 if(['unknown','attach','startup','ack'].includes(scenario)){assert.equal(__comandosBootState,'failed',JSON.stringify(__comandosBootReport));assert.notEqual(global.__comandosReady,true);assert.notEqual(global.__comandosAttached,true);assert(__comandosBootReport.failed.length);assert.equal(calls.filter(c=>c.path==='/web/ready').length,scenario==='ack'?1:0);if(scenario==='unknown')assert.equal(calls.length,0);if(scenario==='attach')assert.equal(calls.some(c=>c.path==='/conf'),false);assert.deepEqual(errors,[]);assert.equal(__comandosBootReport.failed[0].id,{unknown:'missing-component',attach:'ui-general',startup:'ui-general',ack:'@ready'}[scenario]);console.log('PASS native blocks '+scenario,JSON.stringify(__comandosBootReport.failed));return;}
 assert.deepEqual(contentLoads,[],'native startup must never fetch news/analytics payload');assert(!calls.some(c=>c.path.startsWith('/news/')||['/analytics/week','/notices','/notices/watch','/notifs/count','/presence','/pomodoro','/chains','/commands/catalog','/credits/openrouter','/fs/explorer'].includes(c.path)));assert.notEqual(global.__comandosContentReady,true);
 assert.equal(__comandosBootState,'ready',JSON.stringify(__comandosBootReport));assert.equal(__comandosReady,true);assert.equal(__comandosAttached,true);assert.deepEqual(__comandosBootReport.failed,[]);assert.deepEqual([...__comandosBootReport.mounted].sort(),[...ids].sort());assert(global.__comandosStartup instanceof Promise);assert.equal(doc.getElementById('poll').listenerCount('input'),2);assert.equal(typeof doc.getElementById('tab-prev').onclick,'function');assert.equal(typeof global.NewsReader,'object');await __comandosStartup;for(const p of(['/conf','/remote-state','/prefs','/ssh','/state']))assert(calls.some(c=>c.path===p),'startup route '+p);const ready=calls.filter(c=>c.path==='/web/ready');assert.equal(ready.length,1);assert.deepEqual([...JSON.parse(ready[0].options.body).mounted].sort(),[...ids].sort());if(scenario==='late'){const oldRender=AnalyticsRender,oldNews=NewsReader;assert.throws(()=>AnalyticsRender.create({},{}),/not initialized/);await wasm.prepare_content('analytics');assert.equal(AnalyticsRender,oldRender);assert.equal(NewsReader,oldNews);const legacyNews=require(path.resolve(__dirname,'../../../dash/news-reader.js'));const find=doc.getElementById;doc.getElementById=()=>null;const originalNamespace=legacyNews.install();doc.getElementById=find;assert.equal(NewsReader.instance.close(),originalNamespace.close(),'empty reader close return contract');for(const [name,args] of [['statusLabel',['published']],['statusLabel',['failed']],['safeHref',['https://example.invalid/a']],['safeHref',['javascript:alert(1)']],['clampShare',[0.4]],['inlineText',['<b>& \ud83d\ude00']]])assert.deepEqual(NewsReader[name](...args),legacyNews[name](...args),name);const fixtures=path.resolve(__dirname,'../../../tests/fixtures/analytics');const refs=JSON.parse(fs.readFileSync(path.join(fixtures,'reference.json')));for(const [key,want] of Object.entries(refs)){const [model,tab,device]=key.split('|');const data=JSON.parse(fs.readFileSync(path.join(fixtures,model+'.json')));assert.equal(AnalyticsRender.create(data,{phone:device==='phone'}).html(tab),want,key);}assert.equal(typeof NewsReader.createRenderer,'function');assert.equal(typeof ComandosUISounds.createUISounds,'function');console.log('PASS companion synchronous renderer golden contracts and original component factories');}if(scenario==='content-failed'||scenario==='content-missing'){
  {const opened=NewsReader.instance.open('2026-10-06@08:00');assert(opened instanceof Promise);await assert.rejects(opened,/unavailable/);assert.equal(__comandosFeatureReports.news.state,'failed');}
  assert.equal(contentLoads.length,1);assert.equal(__comandosReady,true);assert.equal(__comandosBootState,'ready');assert(!calls.some(c=>c.path.startsWith('/news/')||['/analytics/week','/notices','/notices/watch','/notifs/count','/presence','/pomodoro','/chains','/commands/catalog','/credits/openrouter','/fs/explorer'].includes(c.path)));
 }
 if(scenario==='news-use'||scenario==='news-cancel'||scenario==='news-deeplink'){
  const button=doc.getElementById('btn-news');assert.equal(button.listenerCount('click'),1);
  const original=NewsReader.instance;let opened;
  if(scenario==='news-deeplink'){const deferred=timers.find(([f,ms])=>ms===400);assert(deferred);deferred[0]();await settle();}
  else{opened=original.open('2026-10-06@08:00');assert(opened instanceof Promise);await settle();}
  if(scenario==='news-cancel'){assert.equal(original.close(),undefined);finishContent();await opened;await settle();assert(!calls.some(c=>c.path.startsWith('/news/')));}
  else{if(opened)await opened;await settle();assert(calls.some(c=>c.path==='/news/editions'));assert.equal(calls.filter(c=>c.path==='/news/edition').length,1);assert.equal(NewsReader.instance.reader.element.hidden,false);NewsReader.instance.close();assert.equal(NewsReader.instance.reader.element.hidden,true);assert.equal(NewsReader.instance.toggle(),undefined);await settle();assert.equal(NewsReader.instance.reader.element.hidden,false);NewsReader.instance.close();}
  assert.equal(contentLoads.length,1);assert.equal(button.listenerCount('click'),1,'cold listener must be replaced exactly once');
 }
 if(scenario==='news-desktop'){
  assert.equal(ONLY_PANEL,'news');
  for(const [fire,ms] of timers.slice())if(ms===0)fire();
  await settle();
  assert.equal(__comandosFeatureReports.news.state,'ready');
  assert(calls.some(c=>c.path==='/news/editions'));
  assert(calls.some(c=>c.path==='/web/markdown'),'published stories must exercise authenticated Markdown rendering');
  const reader=NewsReader.instance.reader;
  assert(reader.element.classList.contains('nr-desktop'));
  assert.equal(reader.element.hidden,false);
  assert(reader.element.querySelector('.nr-edition').textContent.includes('Contenido protegido'));
  await reader.setTerminal(true);
  reader.close();
  assert(bridgeMessages.some(m=>m.type==='reader'&&m.action==='terminal'&&m.on===true));
  assert(bridgeMessages.some(m=>m.type==='reader'&&m.action==='close'));
 }
 if(scenario==='sound-gesture'){
  let gesture=false;const audio=[];
  const param=()=>({value:0,cancelScheduledValues(){},setValueAtTime(){},linearRampToValueAtTime(){}});
  global.AudioContext=class{
    constructor(){audio.push(['context',gesture]);this.state='suspended';this.currentTime=0;this.sampleRate=8000;this.destination={};}
    resume(){audio.push(['resume',gesture]);this.state='running';return Promise.resolve();}
    close(){audio.push(['close']);return Promise.resolve();}
    createGain(){return{gain:param(),connect(){},disconnect(){}};}
    createBuffer(channels,length,rate){return{getChannelData:()=>new Float32Array(length)};}
    createBufferSource(){const listeners=new Map();return{connect(){},disconnect(){},start(){audio.push(['start'])},stop(){audio.push(['stop']);listeners.get('ended')?.()},addEventListener:(t,f)=>listeners.set(t,f),removeEventListener:t=>listeners.delete(t)};}
  };
  assert.equal(uiSounds.setEnabled(true),true);assert.equal(uiSounds.unlock({isTrusted:false}),false);assert.deepEqual(audio,[]);
  gesture=true;assert.equal(uiSounds.unlock({isTrusted:true}),true);assert.deepEqual(audio,[['context',true],['resume',true]],'context creation/resume must occur inside the original trusted gesture');gesture=false;
  await settle();const handle=uiSounds.play('focus-start',{eventId:'private-event'});assert(handle&&!(handle instanceof Promise));assert.equal(typeof handle.stop,'function');assert(handle.ended instanceof Promise);assert.equal(uiSounds.play('focus-start',{eventId:'private-event'}),null);handle.stop();await handle.ended;
  document.visibilityState='hidden';assert.equal(uiSounds.play('success'),null);document.visibilityState='visible';assert.equal(uiSounds.play('loading'),null);await uiSounds.dispose();assert.equal(contentLoads.length,0,'audio must never import news/analytics');
 }
 if(scenario==='http-native'){
  assert.equal(WEBTERM,true,'native HTTP dashboard must use its same-origin terminal');
  assert.equal(TERM_BASE,location.origin+'/term');
  assert(doc.body.classList.contains('app'),'remote/native HTTP layout must include sessions');
  assert.equal(typeof doc.getElementById('tab-open').onclick,'function');
  S.list=Array.from({length:25},(_,i)=>({session:'term-fixture-'+i,project:'Session '+i,agent:'codex',alive:true,tabbed:true,status:'idle'}));
  doc.getElementById('tab-open').onclick();
  assert.equal(doc.querySelectorAll('#sw-list .sw-row').length,25,'all sessions must not truncate at twelve');
  assert(!doc.getElementById('sw-ov').classList.contains('hidden'),'all sessions button opens list');
 }
 if(scenario==='sidebar-views'){
  const root=makeEl('div',{},doc.body);doc.body.appendChild(root);
  let quotaPaints=0;const sidebar=ComandosCommandSidebar.createCommandSidebar({root,storage:localStorage,api:async()=>({connected:false}),renderLimits:slot=>{quotaPaints++;slot.textContent='Codex main 23%';}});
  sidebar.render();
  for(const gone of['.sec-terms','.cs-footer','.cs-more','.et-foot','.cs-explorer','[data-sidebar-view="files"]','[data-sheet="cmds"]','[data-sheet="chains"]'])assert.equal(root.querySelector(gone),null,'retired sidebar piece '+gone);
  assert.equal(root.querySelector('.cs-feed').hidden,false,'sessions feed is the default view');assert.equal(root.querySelector('.et-lim').hidden,true,'usage hidden by default');assert.equal(quotaPaints,0,'no quota work while Consumos is hidden');
  root.querySelector('[data-sidebar-view="usage"]').click();assert.equal(root.querySelector('.et-lim').hidden,false,'Consumos tab shows usage');assert.equal(root.querySelector('.cs-feed').hidden,true);assert(quotaPaints>0,'quota renderer called on Consumos');
  assert.equal(root.querySelector('.cs-empty-terms').style.flex,'1 1 auto','feed takes the freed height');
  root.querySelector('[data-sheet="srv"]').click();assert.equal(root.querySelector('.cs-sheet').hidden,false,'servers sheet opens');
 }
 if(scenario==='app-bridge'){
  const cold=NewsReader.instance;assert.equal(cold.open('2026-10-06@08:00'),undefined);assert.equal(cold.close(),undefined);assert.equal(cold.toggle(),undefined);
  const nativeMessages=bridgeMessages.filter(m=>m.type==='reader');bridgeMessages.length=0;
  const legacy=require(path.resolve(__dirname,'../../../dash/news-reader.js')).install();legacy.open('2026-10-06@08:00');legacy.close();legacy.toggle();
  assert.deepEqual(nativeMessages,bridgeMessages.filter(m=>m.type==='reader'));assert.equal(contentLoads.length,0,'native app bridge does not need the web reader');
 }
 const before=[calls.length,timers.length,doc.listenerCount('click')];wasm.boot('ignored');await settle();assert.deepEqual([calls.length,timers.length,doc.listenerCount('click')],before);assert.deepEqual(errors,[]);console.log('PASS native '+scenario+': 48 mounts, complete startup, ACK and idempotency');
})().catch(e=>{console.error(e);process.exitCode=1});
