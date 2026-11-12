//! General controls, operator globals and page-wide coordinator wiring.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_ui() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_ui() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_globals() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_analytics() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach_analytics() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub use web::{attach_analytics, attach_ui, mount_analytics, mount_globals, mount_ui};
#[cfg(target_arch = "wasm32")]
mod web {
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Function, JsString, Promise, Reflect};
    use serde_json::json;
    use wasm_bindgen::{JsCast, JsValue};
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn exported(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        invoke(&global(name), args)
    }
    fn state() -> JsValue {
        invoke(&global("__comandosState"), &[]).unwrap_or_else(|_| global("S"))
    }
    fn new(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        Reflect::construct(
            &global(name).dyn_into::<Function>()?,
            &args.iter().cloned().collect::<Array>(),
        )
    }
    fn contains(el: &JsValue, name: &str) -> Result<bool, JsValue> {
        call(&get(el, "classList"), "contains", &[name.into()]).map(|v| truthy(&v))
    }
    fn dispatch(el: &JsValue) -> Result<(), JsValue> {
        call(
            el,
            "dispatchEvent",
            &[new(
                "Event",
                &["input".into(), from_json(&json!({"bubbles":true}))?],
            )?],
        )?;
        Ok(())
    }
    fn translated(es: String, en: String) -> Result<JsValue, JsValue> {
        exported("tf", &[es.into(), en.into()])
    }
    fn toast_error(err: JsValue) -> Result<(), JsValue> {
        exported("toast", &[get(&err, "message"), true.into()])?;
        Ok(())
    }
    fn raw_callback(
        f: impl std::future::Future<Output = Result<(), JsValue>> + 'static,
    ) -> JsValue {
        promise(async move {
            if let Err(err) = f.await {
                toast_error(err)?;
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn bind_switch(selector: JsValue, key: JsValue, cb: JsValue) -> Result<(), JsValue> {
        let button = query(&doc(), &string(&selector));
        let cfg = get(&state(), "cfg");
        let name = string(&key);
        let value = get(&cfg, &name);
        classes(&button, "on", truthy(&value));
        attr(
            &button,
            "aria-checked",
            if truthy(&value) { "true" } else { "false" },
        );
        listen(
            &button.clone(),
            "click",
            function(move |_| {
                let cfg = get(&state(), "cfg");
                let value = !truthy(&get(&cfg, &name));
                set(&cfg, &name, &value.into())?;
                exported("saveCfg", &[])?;
                classes(&button, "on", value);
                attr(
                    &button,
                    "aria-checked",
                    if value { "true" } else { "false" },
                );
                let owner = call(&button, "closest", &[".switch".into()])?;
                let label = if truthy(&owner) {
                    get(&query(&owner, "label"), "textContent")
                } else {
                    JsValue::UNDEFINED
                };
                let label = if truthy(&label) {
                    string(&label)
                } else {
                    name.clone()
                };
                let status = translated(
                    if value { "activado" } else { "apagado" }.into(),
                    if value { "on" } else { "off" }.into(),
                )?;
                exported("toast", &[format!("{label}: {}", string(&status)).into()])?;
                if truthy(&cb) {
                    invoke(&cb, &[value.into()])?;
                }
                Ok(Promise::resolve(&JsValue::UNDEFINED).into())
            }),
        );
        Ok(())
    }
    pub fn mount_ui() -> Result<(), JsValue> {
        publish("bindSwitch", |a| {
            bind_switch(a.get(0), a.get(1), a.get(2)).map(|_| JsValue::UNDEFINED)
        })
    }
    pub fn attach_ui() -> Result<(), JsValue> {
        listen(
            &id("btn-settings"),
            "click",
            function(|_| {
                let el = id("settings");
                classes(&el, "open", !contains(&el, "open")?);
                for name in ["servers", "remote"] {
                    classes(&id(name), "open", false);
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &doc(),
            "click",
            function(|a| {
                let target = get(&a.get(0), "target");
                for (name, trigger) in [
                    ("settings", "btn-settings"),
                    ("servers", "ssh-manage"),
                    ("remote", "btn-remote"),
                ] {
                    let el = id(name);
                    if !contains(&el, "modal")?
                        && contains(&el, "open")?
                        && !truthy(&call(&el, "contains", std::slice::from_ref(&target))?)
                        && get(&target, "id").as_string().as_deref() != Some(trigger)
                    {
                        classes(&el, "open", false);
                    }
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        for modal in all(&doc(), ".modal") {
            listen(
                &modal.clone(),
                "click",
                function(move |a| {
                    if js_sys::Object::is(&get(&a.get(0), "target"), &modal) {
                        classes(&modal, "open", false);
                    }
                    Ok(JsValue::UNDEFINED)
                }),
            );
        }
        listen(
            &doc(),
            "click",
            function(|a| {
                let btn = call(
                    &get(&a.get(0), "target"),
                    "closest",
                    &["[data-close]".into()],
                )?;
                if truthy(&btn) {
                    let el = id(&string(&get(&get(&btn, "dataset"), "close")));
                    if truthy(&el) {
                        classes(&el, "open", false);
                    }
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &doc(),
            "keydown",
            function(|a| {
                if get(&a.get(0), "key").as_string().as_deref() == Some("Escape") {
                    for modal in all(&doc(), ".modal.open") {
                        classes(&modal, "open", false);
                    }
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        let vol = id("vol");
        set(&js_sys::global(), "vol", &vol)?;
        listen(
            &vol.clone(),
            "change",
            function(move |_| {
                let value = string(&get(&vol, "value"));
                let body = from_json(&json!({"key":"VOLUME","value":value}))?;
                let requested = exported("api", &["/conf-set".into(), body]);
                let vol = vol.clone();
                Ok(raw_callback(async move {
                    wait(requested).await?;
                    let value = string(&get(&vol, "value"));
                    exported(
                        "toast",
                        &[translated(
                            format!("Volumen {value}% (todo el sistema)"),
                            format!("Volume {value}% (system-wide)"),
                        )?],
                    )?;
                    exported("loadConf", &[])?;
                    Ok(())
                }))
            }),
        );
        listen(
            &id("sw-voice"),
            "click",
            function(|_| {
                let button = id("sw-voice");
                let on = !contains(&button, "on")?;
                let body = from_json(&json!({"key":"SPEAK_DONE","value":if on{"1"}else{"0"}}))?;
                let requested = exported("api", &["/conf-set".into(), body]);
                Ok(raw_callback(async move {
                    wait(requested).await?;
                    wait(exported(
                        "api",
                        &[
                            "/conf-set".into(),
                            from_json(
                                &json!({"key":"SPEAK_ATTENTION","value":if on{"1"}else{"0"}}),
                            )?,
                        ],
                    ))
                    .await?;
                    classes(&button, "on", on);
                    attr(&button, "aria-checked", if on { "true" } else { "false" });
                    exported(
                        "toast",
                        &[translated(
                            format!("Voz: {}", if on { "activada" } else { "apagada" }),
                            format!("Voice: {}", if on { "on" } else { "off" }),
                        )?],
                    )?;
                    Ok(())
                }))
            }),
        );
        for (name, kind) in [
            ("test-voice", "voice"),
            ("test-chime", "chime"),
            ("test-done", "done"),
        ] {
            listen(
                &id(name),
                "click",
                function(move |_| {
                    let requested =
                        exported("api", &["/test".into(), from_json(&json!({"kind":kind}))?]);
                    Ok(raw_callback(async move {
                        wait(requested).await?;
                        Ok(())
                    }))
                }),
            );
        }
        bind_switch(
            "#sw-notif".into(),
            "notif".into(),
            function(|a| {
                let notify = global("Notification");
                let on = truthy(&a.get(0));
                let requested =
                    if on && get(&notify, "permission").as_string().as_deref() != Some("granted") {
                        Some(call(&notify, "requestPermission", &[]))
                    } else {
                        None
                    };
                Ok(promise(async move {
                    if let Some(requested) = requested {
                        let permission = wait(requested).await?;
                        if permission.as_string().as_deref() != Some("granted") {
                            set(&get(&state(), "cfg"), "notif", &false.into())?;
                            exported("saveCfg", &[])?;
                            classes(&id("sw-notif"), "on", false);
                            exported(
                                "toast",
                                &["El navegador nego el permiso".into(), true.into()],
                            )?;
                        }
                    }
                    Ok(JsValue::UNDEFINED)
                }))
            }),
        )?;
        let poll = id("poll");
        set(&js_sys::global(), "poll", &poll)?;
        set(&poll, "value", &get(&get(&state(), "cfg"), "poll"))?;
        listen(
            &poll.clone(),
            "input",
            function(move |_| {
                let value = get(&poll, "value");
                set(&get(&state(), "cfg"), "poll", &number(&value).into())?;
                set(&id("poll-v"), "textContent", &value)?;
                exported("saveCfg", &[])?;
                exported("arm", &[])?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        set(
            &id("poll-v"),
            "textContent",
            &get(&get(&state(), "cfg"), "poll"),
        )?;
        listen(
            &js_sys::global(),
            "message",
            global("handleTermFrameMessage"),
        );
        listen(
            &js_sys::global(),
            "pagehide",
            global("restoreAllTermInteractions"),
        );
        listen(
            &js_sys::global(),
            "pageshow",
            global("handleTermInteractionsPageShow"),
        );
        call(
            &js_sys::global(),
            "setInterval",
            &[
                function(|_| {
                    let date = new("Date", &[])?;
                    let h = number(&call(&date, "getHours", &[])?) as u32;
                    let m = number(&call(&date, "getMinutes", &[])?) as u32;
                    set(
                        &id("clock"),
                        "textContent",
                        &format!("{h:02}:{m:02}").into(),
                    )?;
                    Ok(JsValue::UNDEFINED)
                }),
                1000.into(),
            ],
        )?;
        for button in all(&doc(), ".lang-btn") {
            listen(
                &button.clone(),
                "click",
                function(move |_| {
                    let body = object();
                    set(&body, "key", &"CC_LANG".into())?;
                    set(&body, "value", &get(&get(&button, "dataset"), "lang"))?;
                    let requested = exported("api", &["/conf-set".into(), body]);
                    Ok(raw_callback(async move {
                        wait(requested).await?;
                        call(&global("location"), "reload", &[])?;
                        Ok(())
                    }))
                }),
            );
        }
        let conf = exported("api", &["/conf".into()]);
        let startup = promise(async move {
            if let Ok(c) = wait(conf).await
                && get(&c, "_lang").as_string().as_deref() == Some("en")
            {
                set(&js_sys::global(), "L", &"en".into())?;
            }
            exported("applyI18n", &[])?;
            if global("L").as_string().as_deref() == Some("en") {
                set(
                    &js_sys::global(),
                    "LABEL",
                    &from_json(
                        &json!({"waiting":"waiting for you","done":"finished","working":"working","idle":"alive, idle","dead":"off"}),
                    )?,
                )?;
            }
            exported("initApp", &[])?;
            exported("mountCommandSidebar", &[])?;
            wait(exported("loadRemote", &[])).await?;
            call(&exported("loadPrefs", &[])?, "then", &[global("tick")])?;
            exported("loadSsh", &[])?;
            exported("loadConf", &[])?;
            exported("arm", &[])?;
            Ok(JsValue::UNDEFINED)
        });
        set(&js_sys::global(), "__comandosStartup", &startup)?;
        Ok(())
    }
    pub fn mount_globals() -> Result<(), JsValue> {
        publish("opFavorite", |a| {
            let requested = exported("setSessionFavorite", &[a.get(0), a.get(1)]);
            Ok(promise(async move {
                wait(requested).await?;
                Ok(JsValue::UNDEFINED)
            }))
        })?;
        publish("setPollSeconds", |a| {
            let el = id("poll");
            if truthy(&el) {
                set(&el, "value", &a.get(0))?;
                dispatch(&el)?;
            }
            Ok(JsValue::UNDEFINED)
        })?;
        publish("setBrowserNotifications", |a| {
            let el = id("sw-notif");
            if truthy(&el) && contains(&el, "on")? != truthy(&a.get(0)) {
                call(&el, "click", &[])?;
            }
            Ok(JsValue::UNDEFINED)
        })?;
        publish("closeAllPanels", |_| {
            for modal in all(&doc(), ".modal.open") {
                classes(&modal, "open", false);
            }
            exported("closeModelMenus", &[])?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("swOpenWith", |a| {
            exported("swOpen", &[])?;
            let el = id("sw-in");
            if truthy(&el) {
                set(
                    &el,
                    "value",
                    &if truthy(&a.get(0)) {
                        a.get(0)
                    } else {
                        "".into()
                    },
                )?;
                dispatch(&el)?;
            }
            Ok(JsValue::UNDEFINED)
        })?;
        publish("nsOpenPrefilled", |a| {
            let opened = exported("nsOpen", &[])?;
            call(
                &Promise::resolve(&opened).into(),
                "then",
                &[function(move |_| {
                    let cwd = a.get(0);
                    if truthy(&cwd) {
                        let el = id("ns-cwd");
                        if truthy(&el) {
                            set(&el, "value", &cwd)?;
                            dispatch(&el)?;
                        }
                    }
                    for (selector, i) in [
                        ("#ns-harness", 1),
                        ("#ns-motor", 2),
                        ("#ns-model", 3),
                        ("#ns-effort", 4),
                    ] {
                        let val = a.get(i);
                        if !truthy(&val) {
                            continue;
                        }
                        for el in all(&doc(), &format!("{selector} button")) {
                            let text: JsValue =
                                JsString::from(get(&el, "textContent")).trim().into();
                            if get(&get(&el, "dataset"), "id") == val || text == val {
                                call(&el, "click", &[])?;
                                break;
                            }
                        }
                    }
                    if let Some(danger) = a.get(5).as_bool() {
                        let el = id("ns-danger");
                        if truthy(&el) && contains(&el, "on")? != danger {
                            call(&el, "click", &[])?;
                        }
                    }
                    Ok(JsValue::UNDEFINED)
                })],
            )?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn mount_analytics() -> Result<(), JsValue> {
        let original = global("setSplitLeft");
        publish("setSplitLeft", move |a| {
            if a.length() >= 2 {
                return invoke(&original, &[a.get(0), a.get(1), a.get(2)]);
            }
            call(
                &global("localStorage"),
                "setItem",
                &["cc-split-left".into(), string(&a.get(0)).into()],
            )?;
            exported("restoreSplitLeft", &[])?;
            exported("applyAppLayout", &[])?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("reloadDashboard", |_| {
            call(&global("location"), "reload", &[])?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("focusVisibleTerm", |_| {
            let active = global("activeTerm");
            let frame = if truthy(&active) {
                get(&call(&global("openTerms"), "get", &[active])?, "frame")
            } else {
                JsValue::UNDEFINED
            };
            let window = get(&frame, "contentWindow");
            if !frame.is_null()
                && !frame.is_undefined()
                && !window.is_null()
                && !window.is_undefined()
            {
                call(
                    &window,
                    "postMessage",
                    &[
                        from_json(
                            &json!({"source":"comandos","type":"toolbar","term":{"type":"focus"}}),
                        )?,
                        get(&global("location"), "origin"),
                    ],
                )?;
            }
            Ok(JsValue::UNDEFINED)
        })?;
        // State/translation/icons are already published by their own regions.
        publish("__comandosNewsTerminal", |_| {
            let out = object();
            set(&out, "base", &global("TERM_BASE"))?;
            set(&out, "theme", &global("curTheme"))?;
            Ok(out)
        })
    }
    pub fn attach_analytics() -> Result<(), JsValue> {
        listen(
            &doc(),
            "keydown",
            function(|a| {
                let e = a.get(0);
                let key = get(&e, "key");
                if truthy(&get(&e, "ctrlKey"))
                    && truthy(&get(&e, "shiftKey"))
                    && (key.as_string().as_deref() == Some("K")
                        || key.as_string().as_deref() == Some("k"))
                {
                    call(&e, "preventDefault", &[])?;
                    exported(
                        if truthy(&get(&global("snipDlg"), "open")) {
                            "closeSnippets"
                        } else {
                            "openSnippets"
                        },
                        &[],
                    )?;
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        if Reflect::has(&global("navigator"), &"serviceWorker".into())? {
            let worker = get(&global("navigator"), "serviceWorker");
            let mode = call(
                &global("document"),
                "querySelector",
                &["meta[name=\"comandos-web-mode\"]".into()],
            )?;
            let path = if get(&mode, "content") == "native" {
                "/sw.js?native=1"
            } else {
                "/sw.js"
            };
            let registered = call(&worker, "register", &[path.into()])?;
            call(
                &registered,
                "catch",
                &[function(|_| Ok(JsValue::UNDEFINED))],
            )?;
        }
        Ok(())
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
#[allow(clippy::unwrap_used)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;
    #[wasm_bindgen(inline_js = r#"
import vm from 'node:vm';
function ui_context(){
 const ctx={calls:[],timers:[],resize:[],events:{},nodes:{},S:{cfg:{notif:false,poll:2}},L:'es',LABEL:{waiting:'Esperando respuesta'},location:{search:'?demo=PrO-1neon',origin:'https://private.invalid',reload(){ctx.calls.push('reload')}},navigator:{serviceWorker:{register(p){ctx.calls.push(['register',p]);return Promise.resolve()}}},Notification:{permission:'default',requestPermission(){return Promise.resolve('denied')}},Date,URLSearchParams,Promise,Event,JSON,CSS:{escape:x=>String(x).replace(/["\\]/g,'\\$&')}};
 const node=(id='',cls=[])=>{const e={id,value:'',dataset:{},textContent:'',style:{},events:{},classList:new Set(cls),children:[],addEventListener(n,f){(this.events[n]??=[]).push(f)},dispatchEvent(e){for(const f of this.events[e.type]??[])f(e);ctx.calls.push(['dispatch',this.id,e.type,e.bubbles]);return true},click(){this.clicked=(this.clicked??0)+1;for(const f of this.events.click??[])f({target:this});},setAttribute(k,v){this[k]=String(v)},closest(s){return this.closestMap?.[s]??null},querySelector(){return this.label??null},contains(t){return this===t},getBoundingClientRect(){return {width:this.width??900}}};e.classList.contains=e.classList.has;e.classList.toggle=(k,on)=>{if(arguments.length===1)on=!e.classList.has(k);return on?e.classList.add(k):e.classList.delete(k)};return e};
 for(const id of ['btn-settings','btn-usage','settings','servers','remote','usage','vol','sw-voice','test-voice','test-chime','test-done','sw-notif','poll','poll-v','clock','sw-in','ns-cwd','ns-danger','an-root'])ctx.nodes[id]=node(id,['settings','servers','remote','usage'].includes(id)?['modal']:[]);
 const lang=node('lang');lang.dataset.lang='en';ctx.lang=lang;
 ctx.modals=['settings','servers','remote','usage'].map(k=>ctx.nodes[k]);ctx.docEvents={};ctx.selectors=[];
 ctx.pick={};for(const k of ['harness','motor','model','effort']){const e=node(k);e.dataset.id=k+'-id';e.textContent=' '+k+' label ';ctx.pick['#ns-'+k+' button']=[e]};
 ctx.document={body:node('body'),getElementById:k=>ctx.nodes[k]??null,querySelector:s=>{ctx.selectors.push(s);if(s.startsWith('.nf2-x')||s.startsWith('[data-act='))return ctx.nf??null;return ctx.nodes[s.slice(1)]??null},querySelectorAll:s=>s==='.modal'?ctx.modals:s==='.modal.open'?ctx.modals.filter(e=>e.classList.has('open')):s==='.lang-btn'?[ctx.lang]:ctx.pick[s]??[],addEventListener(n,f){(ctx.docEvents[n]??=[]).push(f)}};
 ctx.$=s=>ctx.document.querySelector(s);ctx.__comandosState=()=>ctx.S;ctx.addEventListener=(n,f)=>{(ctx.events[n]??=[]).push(f)};ctx.setInterval=(f,ms)=>{ctx.timers.push({f,ms});return ctx.timers.length};ctx.ResizeObserver=class{constructor(f){ctx.resize.push(this);this.f=f}observe(el){this.el=el}};
 ctx.tf=(es,en)=>ctx.L==='en'?en:es;ctx.toast=(...a)=>ctx.calls.push(['toast',...a]);ctx.saveCfg=()=>ctx.calls.push('saveCfg');ctx.applyI18n=()=>ctx.calls.push('applyI18n');ctx.inApp=()=>!!ctx.webkit;
 ctx.api=(path,body)=>{ctx.calls.push(['api',path,body]);return path==='/conf'?Promise.resolve({_lang:'en'}):Promise.resolve({ok:true})};
 for(const name of ['initApp','mountCommandSidebar','mountChainPage','loadSsh','loadConf','arm','tick','handleTermFrameMessage','restoreAllTermInteractions','handleTermInteractionsPageShow','closeModelMenus','restoreSplitLeft','applyAppLayout'])ctx[name]=(...a)=>ctx.calls.push([name,...a]);
 ctx.loadRemote=()=>{ctx.calls.push('loadRemote');return Promise.resolve()};ctx.loadPrefs=()=>{ctx.calls.push('loadPrefs');return Promise.resolve()};ctx.setSessionFavorite=(...a)=>{ctx.calls.push(['favorite',...a]);return Promise.resolve('ignored result')};ctx.swOpen=()=>ctx.calls.push('swOpen');ctx.nsOpen=()=>{ctx.calls.push('nsOpen');return Promise.resolve()};ctx.closeSnippets=()=>ctx.calls.push('closeSnippets');ctx.openSnippets=()=>ctx.calls.push('openSnippets');ctx.snipDlg={open:false};
 const store=new Map();ctx.localStorage={getItem:k=>store.get(k)??null,setItem:(k,v)=>store.set(k,String(v))};ctx.analyticsCalls=[];ctx.Analytics={create(el,opts){ctx.analyticsCalls.push(['create',el.id,opts.tab]);ctx.analyticsOpts=opts;return {open(t){ctx.analyticsCalls.push(['open',t])},load(){ctx.analyticsCalls.push('load')},paint(){ctx.analyticsCalls.push('paint')}}}};
 ctx.setSplitLeft=(...a)=>{ctx.calls.push(['originalSplit',...a]);return 'original result'};ctx.openTerms=new Map();ctx.activeTerm=null;ctx.TERM_BASE='https://private.invalid:7681';ctx.curTheme='neon';ctx.window=ctx;ctx.node=node;return ctx;
}
export function ui_fixture(){const ctx=ui_context();globalThis.ui=ctx;delete globalThis.navigator;Object.assign(globalThis,ctx);globalThis.window=globalThis;globalThis.__comandosState=()=>ctx.S;globalThis.inApp=()=>!!globalThis.webkit;globalThis.tf=(es,en)=>globalThis.L==='en'?en:es;}
export async function ui_contracts(source){
 const eq=(a,b,msg)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(msg+': '+JSON.stringify(a)+' != '+JSON.stringify(b));};const ok=(v,m)=>{if(!v)throw Error(m)};const drain=async()=>{for(let i=0;i<32;i++)await Promise.resolve();};
 await drain();
 const ctx=ui_context();ctx.openAnalytics=()=>ctx.calls.push('openAnalytics');vm.createContext(ctx);const cut=(a,b)=>source.slice(source.indexOf(a),source.indexOf(b,source.indexOf(a)));vm.runInContext(cut('// ---------- UI general ----------','// ---------- snippets (v0) ----------'),ctx);vm.runInContext(cut('// ---------- Analytics (grill 1–2 oct): Cuentas · Comparar · Pomodoro ----------','</script>'),ctx);await drain();
 eq(ui.calls,ctx.calls.filter(x=>!(Array.isArray(x)&&x[0]==='mountChainPage')),'original startup order without the retired chains page');eq(LABEL,{waiting:'waiting for you',done:'finished',working:'working',idle:'alive, idle',dead:'off'},'language labels');eq(ui.nodes.poll.value,2,'poll initialized');eq(ui.nodes['poll-v'].textContent,2,'poll label');
 const event=async(id,type)=>{let results=ui.nodes[id].events[type].map(f=>f({target:ui.nodes[id]}));await Promise.all(results);await drain()};
 ui.nodes.vol.value='42';await event('vol','change');eq(ui.calls.slice(-3),[['api','/conf-set',{key:'VOLUME',value:'42'}],['toast','Volume 42% (system-wide)'],['loadConf']],'volume write');
 await event('sw-voice','click');eq(ui.calls.filter(x=>Array.isArray(x)&&x[0]==='api'&&String(x[2]?.key).startsWith('SPEAK')).slice(-2),[['api','/conf-set',{key:'SPEAK_DONE',value:'1'}],['api','/conf-set',{key:'SPEAK_ATTENTION',value:'1'}]],'voice two keys sequential');ok(ui.nodes['sw-voice'].classList.has('on'),'voice on');eq(ui.nodes['sw-voice']['aria-checked'],'true','voice aria');
 for(const [id,kind]of[['test-voice','voice'],['test-chime','chime'],['test-done','done']]){await event(id,'click');eq(ui.calls.slice(-1)[0],['api','/test',{kind}],'test route')}
 await event('sw-notif','click');eq(S.cfg.notif,false,'permission denial resets preference');ok(!ui.nodes['sw-notif'].classList.has('on'),'permission denial removes on');eq(ui.nodes['sw-notif']['aria-checked'],'true','original denial aria behavior retained');
 setPollSeconds(7);eq(S.cfg.poll,7,'operator poll dispatch');eq(ui.nodes['poll-v'].textContent,7,'poll label update');ok(ui.calls.some(x=>Array.isArray(x)&&x[0]==='arm'),'poll armed');await opFavorite('session',true);eq(ui.calls.slice(-1)[0],['favorite','session',true],'favorite delegating');
 setBrowserNotifications(false);eq(ui.nodes['sw-notif'].clicked??0,0,'matching flag no click');setBrowserNotifications(true);eq(ui.nodes['sw-notif'].clicked,1,'mismatch flag click');await drain();
 swOpenWith('needle');eq(ui.nodes['sw-in'].value,'needle','prefilled switcher');ok(ui.calls.some(x=>Array.isArray(x)&&x[0]==='dispatch'&&x[1]==='sw-in'),'switch input dispatch');
 nsOpenPrefilled('/private','harness-id','motor label','model-id','effort label',true);await drain();eq(ui.nodes['ns-cwd'].value,'/private','prefilled cwd');for(const buttons of Object.values(ui.pick))eq(buttons[0].clicked,1,'id or label picks');eq(ui.nodes['ns-danger'].clicked,1,'boolean danger toggle');
 for(const modal of ui.modals)modal.classList.add('open');closeAllPanels();ok(ui.modals.every(m=>!m.classList.has('open')),'all modals close');eq(ui.calls.slice(-1)[0],['closeModelMenus'],'model menu closes');
 ui.nodes.settings.events.click[0]({target:ui.nodes.settings});ui.nodes.settings.classList.add('open');ui.docEvents.keydown[0]({key:'Escape'});ok(!ui.nodes.settings.classList.has('open'),'escape modal');
 ui.nodes['btn-settings'].click();ok(ui.nodes.settings.classList.has('open'),'settings button toggles');
 await Promise.all(ui.lang.events.click.map(f=>f({target:ui.lang})));eq(ui.calls.slice(-2),[['api','/conf-set',{key:'CC_LANG',value:'en'}],'reload'],'language switch');
 setSplitLeft(123);eq(localStorage.getItem('cc-split-left'),'123','split persistence');eq(setSplitLeft(321,false,{min:40}),'original result','legacy multiargument split');eq(ui.calls.slice(-1)[0],['originalSplit',321,false,{min:40}],'split delegate');
 openTerms.set('session',{frame:{contentWindow:{postMessage:(...a)=>ui.calls.push(['frame',...a])}}});globalThis.activeTerm='session';focusVisibleTerm();eq(ui.calls.slice(-1)[0],['frame',{source:'comandos',type:'toolbar',term:{type:'focus'}},location.origin],'frame focus message');eq(__comandosNewsTerminal(),{base:TERM_BASE,theme:curTheme},'live news terminal boundary');
 const keyboard=ui.docEvents.keydown.slice(-1)[0];let prevented=false;keyboard({key:'K',ctrlKey:true,shiftKey:true,preventDefault(){prevented=true}});ok(prevented,'snippets shortcut prevented');eq(ui.calls.slice(-1)[0],'openSnippets','snippets shortcut opens');snipDlg.open=true;keyboard({key:'k',ctrlKey:true,shiftKey:true,preventDefault(){}});eq(ui.calls.slice(-1)[0],'closeSnippets','snippets shortcut closes');
}
"#)]
    extern "C" {
        fn ui_fixture();
        #[wasm_bindgen(catch)]
        async fn ui_contracts(source: &str) -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test(async)]
    async fn general_ui_globals_preserve_original_startup_and_controls() {
        ui_fixture();
        mount_ui().unwrap();
        mount_globals().unwrap();
        mount_analytics().unwrap();
        attach_ui().unwrap();
        attach_analytics().unwrap();
        ui_contracts(include_str!("../../../../dash/index.html"))
            .await
            .unwrap();
    }
}
