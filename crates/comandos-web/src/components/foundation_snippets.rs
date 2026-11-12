//! Complete snippets coordinator: live objects, DOM, CRUD and paste without Enter.
pub fn match_rank(name: bool, tags: bool, body: bool) -> u8 {
    if name {
        3
    } else if tags {
        2
    } else if body {
        1
    } else {
        0
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn mount() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn attach() -> Result<(), wasm_bindgen::JsValue> {
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_outrank_tags_and_body_and_no_match_is_excluded() {
        assert_eq!(match_rank(true, true, true), 3);
        assert_eq!(match_rank(false, true, true), 2);
        assert_eq!(match_rank(false, false, true), 1);
        assert_eq!(match_rank(false, false, false), 0);
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::{attach, mount};
#[cfg(target_arch = "wasm32")]
mod web {
    use super::match_rank;
    use crate::components::web_support::*;
    use comandos_web_dom::port::*;
    use js_sys::{Array, Promise};
    use serde_json::json;
    use std::cmp::Ordering;
    use wasm_bindgen::JsValue;
    fn publish(
        name: &str,
        f: impl Fn(Array) -> Result<JsValue, JsValue> + 'static,
    ) -> Result<(), JsValue> {
        set(&js_sys::global(), name, &function(f))
    }
    fn exported(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        invoke(&global(name), args)
    }
    fn method(value: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let boxed = invoke(&global("Object"), std::slice::from_ref(value))?;
        call(&boxed, name, args)
    }
    fn fallback(value: JsValue) -> JsValue {
        if truthy(&value) { value } else { "".into() }
    }
    fn rows(value: JsValue) -> Array {
        if truthy(&value) {
            Array::from(&value)
        } else {
            Array::new()
        }
    }
    fn snips() -> JsValue {
        global("snipState")
    }
    fn toast_error(prefix: &str, error: JsValue) -> Result<(), JsValue> {
        let message = get(&error, "message");
        exported(
            "toast",
            &[
                if prefix.is_empty() {
                    message
                } else {
                    format!("{prefix}{}", string(&message)).into()
                },
                true.into(),
            ],
        )?;
        Ok(())
    }
    fn safe_action(f: impl std::future::Future<Output = Result<(), JsValue>> + 'static) -> JsValue {
        promise(async move {
            if let Err(e) = f.await {
                toast_error("", e)?;
            }
            Ok(JsValue::UNDEFINED)
        })
    }
    fn create(tag: &str) -> Result<JsValue, JsValue> {
        call(&doc(), "createElement", &[tag.into()])
    }
    fn append(parent: &JsValue, child: &JsValue) -> Result<(), JsValue> {
        call(parent, "appendChild", std::slice::from_ref(child))?;
        Ok(())
    }
    fn button(text: &str) -> Result<JsValue, JsValue> {
        let button = create("button")?;
        set(&button, "className", &"hdr-btn".into())?;
        set(&button, "textContent", &text.into())?;
        Ok(button)
    }
    fn default_session(sessions: JsValue, last: JsValue) -> JsValue {
        let sessions = rows(sessions);
        if truthy(&last) && sessions.iter().any(|s| get(&s, "session") == last) {
            return last;
        }
        if sessions.length() > 0 {
            fallback(get(&sessions.get(0), "session"))
        } else {
            "".into()
        }
    }
    struct Ranked {
        item: JsValue,
        rank: u8,
        updated: f64,
    }
    fn filter(items: JsValue, query: JsValue) -> Result<JsValue, JsValue> {
        let query = method(&method(&fallback(query), "trim", &[])?, "toLowerCase", &[])?;
        let search = truthy(&query);
        let mut ranked = Vec::new();
        for item in Array::from(&items).iter() {
            let rank = if search {
                let name = method(&fallback(get(&item, "name")), "toLowerCase", &[])?;
                let tags = get(&item, "tags");
                let tags = if truthy(&tags) {
                    tags
                } else {
                    Array::new().into()
                };
                let lowered = Array::new();
                for tag in Array::from(&tags).iter() {
                    lowered.push(&method(&tag, "toLowerCase", &[])?);
                }
                let joined = call(&lowered.into(), "join", &[" ".into()])?;
                let body = method(&fallback(get(&item, "body")), "toLowerCase", &[])?;
                match_rank(
                    truthy(&method(&name, "includes", std::slice::from_ref(&query))?),
                    truthy(&method(&joined, "includes", std::slice::from_ref(&query))?),
                    truthy(&method(&body, "includes", std::slice::from_ref(&query))?),
                )
            } else {
                0
            };
            if search && rank == 0 {
                continue;
            }
            let updated = get(&item, "updated_at");
            ranked.push(Ranked {
                item,
                rank,
                updated: if truthy(&updated) {
                    number(&updated)
                } else {
                    0.0
                },
            });
        }
        ranked.sort_by(|a, b| {
            b.rank
                .cmp(&a.rank)
                .then_with(|| b.updated.partial_cmp(&a.updated).unwrap_or(Ordering::Equal))
        });
        let array = Array::new();
        for row in ranked {
            array.push(&row.item);
        }
        Ok(array.into())
    }
    fn render_list() -> Result<(), JsValue> {
        let ul = id("snip-items");
        let filtered = Array::from(&filter(get(&snips(), "items"), get(&snips(), "query"))?);
        set(&ul, "innerHTML", &"".into())?;
        for item in filtered.iter() {
            let li = create("li")?;
            set(&get(&li, "dataset"), "id", &get(&item, "id"))?;
            if get(&item, "id") == get(&snips(), "selectedId") {
                classes(&li, "active", true);
            }
            let name = create("div")?;
            set(&name, "className", &"n".into())?;
            set(&name, "textContent", &get(&item, "name"))?;
            append(&li, &name)?;
            let tags = get(&item, "tags");
            if truthy(&get(&tags, "length")) {
                let line = create("div")?;
                set(&line, "className", &"t".into())?;
                set(&line, "textContent", &call(&tags, "join", &[" · ".into()])?)?;
                append(&li, &line)?;
            }
            listen(
                &li,
                "click",
                function(move |_| {
                    set(&snips(), "selectedId", &get(&item, "id"))?;
                    render_list()?;
                    render_pane()?;
                    Ok(JsValue::UNDEFINED)
                }),
            );
            append(&ul, &li)?;
        }
        if !truthy(&get(&snips(), "selectedId")) && filtered.length() > 0 {
            set(&snips(), "selectedId", &get(&filtered.get(0), "id"))?;
            render_pane()?;
        }
        Ok(())
    }
    fn render_pane() -> Result<(), JsValue> {
        let pane = id("snip-pane");
        let selected = get(&snips(), "selectedId");
        let item = rows(get(&snips(), "items"))
            .iter()
            .find(|x| get(x, "id") == selected);
        let Some(item) = item else {
            set(
                &pane,
                "innerHTML",
                &"<div class=\"snip-empty\">Elige un snippet o crea uno nuevo.</div>".into(),
            )?;
            return Ok(());
        };
        set(&pane, "innerHTML", &"".into())?;
        let heading = create("div")?;
        set(
            &get(&heading, "style"),
            "cssText",
            &"font-size:13px;color:var(--text);font-weight:600".into(),
        )?;
        set(&heading, "textContent", &get(&item, "name"))?;
        append(&pane, &heading)?;
        let tags = get(&item, "tags");
        if truthy(&get(&tags, "length")) {
            let tags_el = create("div")?;
            set(
                &get(&tags_el, "style"),
                "cssText",
                &"display:flex;gap:6px;flex-wrap:wrap".into(),
            )?;
            for tag in Array::from(&tags).iter() {
                let chip = create("span")?;
                set(&chip, "className", &"pill".into())?;
                set(&chip, "textContent", &tag)?;
                append(&tags_el, &chip)?;
            }
            append(&pane, &tags_el)?;
        }
        let pre = create("pre")?;
        set(&pre, "textContent", &get(&item, "body"))?;
        append(&pane, &pre)?;
        let actions = create("div")?;
        set(&actions, "className", &"snip-actions".into())?;
        let label = create("label")?;
        set(&label, "textContent", &"Enviar a:".into())?;
        set(&label, "htmlFor", &"snip-dest".into())?;
        set(
            &get(&label, "style"),
            "cssText",
            &"color:var(--faint);font-size:11px".into(),
        )?;
        append(&actions, &label)?;
        let select = create("select")?;
        set(&select, "id", &"snip-dest".into())?;
        set(
            &select,
            "title",
            &"Sesión destino: el comando aparecerá en el prompt de esa pestaña, esperando tu Enter"
                .into(),
        )?;
        let last = fallback(call(
            &global("localStorage"),
            "getItem",
            &["snippet-last-session".into()],
        )?);
        let sessions = rows(global("snipSessions"));
        let selected = default_session(sessions.clone().into(), last);
        if sessions.length() == 0 {
            let opt = create("option")?;
            set(&opt, "value", &"".into())?;
            set(&opt, "textContent", &"sin sesiones vivas".into())?;
            set(&opt, "disabled", &true.into())?;
            append(&select, &opt)?;
        } else {
            for session in sessions.iter() {
                let opt = create("option")?;
                let key = get(&session, "session");
                set(&opt, "value", &key)?;
                let project = get(&session, "project");
                set(
                    &opt,
                    "textContent",
                    &if truthy(&project) {
                        project
                    } else {
                        key.clone()
                    },
                )?;
                if key == selected {
                    set(&opt, "selected", &true.into())?;
                }
                append(&select, &opt)?;
            }
        }
        append(&actions, &select)?;
        let send = button("Send")?;
        let send_item = item.clone();
        listen(
            &send,
            "click",
            function(move |_| {
                let session = get(&select, "value");
                if !truthy(&session) {
                    exported("toast", &["No hay sesión destino".into(), true.into()])?;
                    return Ok(Promise::resolve(&JsValue::UNDEFINED).into());
                }
                let body = object();
                set(&body, "session", &session)?;
                set(&body, "text", &get(&send_item, "body"))?;
                let requested = exported("api", &["/paste".into(), body]);
                let select = select.clone();
                Ok(safe_action(async move {
                    wait(requested).await?;
                    let session = get(&select, "value");
                    call(
                        &global("localStorage"),
                        "setItem",
                        &["snippet-last-session".into(), session.clone()],
                    )?;
                    exported("toast", &[format!("Pegado en {}", string(&session)).into()])?;
                    Ok(())
                }))
            }),
        );
        append(&actions, &send)?;
        let copy = button("Copy")?;
        let copy_item = item.clone();
        listen(
            &copy,
            "click",
            function(move |_| {
                let copied = call(
                    &get(&global("navigator"), "clipboard"),
                    "writeText",
                    &[get(&copy_item, "body")],
                );
                Ok(promise(async move {
                    match wait(copied).await {
                        Ok(_) => {
                            exported("toast", &["Copiado".into()])?;
                        }
                        Err(e) => {
                            toast_error("No pude copiar: ", e)?;
                        }
                    }
                    Ok(JsValue::UNDEFINED)
                }))
            }),
        );
        append(&actions, &copy)?;
        let edit = button("Edit")?;
        let edit_item = item.clone();
        listen(
            &edit,
            "click",
            function(move |_| {
                editor(edit_item.clone())?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        append(&actions, &edit)?;
        let delete = button("Delete")?;
        listen(
            &delete,
            "click",
            function(move |_| {
                if !truthy(&exported(
                    "confirm",
                    &[format!("¿Borrar \"{}\"?", string(&get(&item, "name"))).into()],
                )?) {
                    return Ok(Promise::resolve(&JsValue::UNDEFINED).into());
                }
                let body = object();
                set(&body, "id", &get(&item, "id"))?;
                let requested = exported("api", &["/snippets/delete".into(), body]);
                let item = item.clone();
                Ok(safe_action(async move {
                    wait(requested).await?;
                    let remain = Array::new();
                    for row in rows(get(&snips(), "items")).iter() {
                        if get(&row, "id") != get(&item, "id") {
                            remain.push(&row);
                        }
                    }
                    set(&snips(), "items", &remain.into())?;
                    set(&snips(), "selectedId", &JsValue::NULL)?;
                    render_list()?;
                    render_pane()?;
                    exported("toast", &["Borrado".into()])?;
                    Ok(())
                }))
            }),
        );
        append(&actions, &delete)?;
        append(&pane, &actions)
    }
    fn editor(item: JsValue) -> Result<(), JsValue> {
        let pane = id("snip-pane");
        set(&pane, "innerHTML", &"".into())?;
        let name_label = create("label")?;
        set(&name_label, "textContent", &"Nombre".into())?;
        let label_style: JsValue =
            "font-size:11px;color:var(--dim);text-transform:uppercase;letter-spacing:.12em".into();
        set(&get(&name_label, "style"), "cssText", &label_style)?;
        let name_input = create("input")?;
        set(
            &name_input,
            "value",
            &if truthy(&item) {
                get(&item, "name")
            } else {
                "".into()
            },
        )?;
        set(&name_input, "placeholder", &"docker-ps".into())?;
        let input_style:JsValue="background:var(--bg);color:var(--text);border:1px solid var(--line);border-radius:6px;padding:6px 10px;font:inherit".into();
        set(&get(&name_input, "style"), "cssText", &input_style)?;
        let tags_label = create("label")?;
        set(&tags_label, "textContent", &"Tags (coma-separados)".into())?;
        set(&get(&tags_label, "style"), "cssText", &label_style)?;
        let tags_input = create("input")?;
        let tags = if truthy(&item) {
            let tags = get(&item, "tags");
            call(
                &if truthy(&tags) {
                    tags
                } else {
                    Array::new().into()
                },
                "join",
                &[", ".into()],
            )?
        } else {
            "".into()
        };
        set(&tags_input, "value", &tags)?;
        set(&tags_input, "placeholder", &"docker, prod".into())?;
        set(&get(&tags_input, "style"), "cssText", &input_style)?;
        let body_label = create("label")?;
        set(&body_label, "textContent", &"Body".into())?;
        set(&get(&body_label, "style"), "cssText", &label_style)?;
        let body_input = create("textarea")?;
        set(
            &body_input,
            "value",
            &if truthy(&item) {
                get(&item, "body")
            } else {
                "".into()
            },
        )?;
        set(&body_input, "rows", &10.into())?;
        set(&get(&body_input,"style"),"cssText",&"background:var(--bg);color:var(--text);border:1px solid var(--line);border-radius:6px;padding:8px 10px;font:inherit;resize:vertical;flex:1;min-height:140px".into())?;
        let actions = create("div")?;
        set(&actions, "className", &"snip-actions".into())?;
        let save = button("Save")?;
        let cancel = button("Cancel")?;
        append(&actions, &save)?;
        append(&actions, &cancel)?;
        for element in [
            &name_label,
            &name_input,
            &tags_label,
            &tags_input,
            &body_label,
            &body_input,
            &actions,
        ] {
            append(&pane, element)?;
        }
        call(&name_input, "focus", &[])?;
        listen(
            &cancel,
            "click",
            function(|_| {
                render_pane()?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &save,
            "click",
            function(move |_| {
                let name = method(&get(&name_input, "value"), "trim", &[])?;
                let body = get(&body_input, "value");
                let parts = method(&get(&tags_input, "value"), "split", &[",".into()])?;
                let tags = Array::new();
                for value in Array::from(&parts).iter() {
                    let trimmed = method(&value, "trim", &[])?;
                    if truthy(&trimmed) {
                        tags.push(&trimmed);
                    }
                }
                if !truthy(&name) {
                    exported("toast", &["Nombre vacío".into(), true.into()])?;
                    return Ok(Promise::resolve(&JsValue::UNDEFINED).into());
                }
                if !truthy(&method(&body, "trim", &[])?) {
                    exported("toast", &["Body vacío".into(), true.into()])?;
                    return Ok(Promise::resolve(&JsValue::UNDEFINED).into());
                }
                let data = object();
                set(&data, "name", &name)?;
                set(&data, "body", &body)?;
                set(&data, "tags", &tags.into())?;
                let updating = truthy(&item);
                if updating {
                    set(&data, "id", &get(&item, "id"))?;
                }
                let requested = exported(
                    "api",
                    &[
                        if updating {
                            "/snippets/update"
                        } else {
                            "/snippets"
                        }
                        .into(),
                        data,
                    ],
                );
                let item = item.clone();
                Ok(safe_action(async move {
                    let saved = wait(requested).await?;
                    let value = get(&saved, "item");
                    let next = Array::new();
                    if !updating {
                        next.push(&value);
                    }
                    for old in rows(get(&snips(), "items")).iter() {
                        next.push(&if updating && get(&old, "id") == get(&item, "id") {
                            value.clone()
                        } else {
                            old
                        });
                    }
                    set(&snips(), "items", &next.into())?;
                    set(&snips(), "selectedId", &get(&value, "id"))?;
                    render_list()?;
                    render_pane()?;
                    exported(
                        "toast",
                        &[if updating { "Actualizado" } else { "Guardado" }.into()],
                    )?;
                    Ok(())
                }))
            }),
        );
        Ok(())
    }
    fn open() -> Result<JsValue, JsValue> {
        call(&global("snipDlg"), "showModal", &[])?;
        let query = id("snip-q");
        if truthy(&query) {
            set(&query, "value", &"".into())?;
            call(&query, "focus", &[])?;
        }
        set(&snips(), "query", &"".into())?;
        let requested = exported("api", &["/snippets".into()]);
        Ok(promise(async move {
            match wait(requested).await {
                Ok(items) => {
                    set(&snips(), "items", &items)?;
                }
                Err(e) => {
                    set(&snips(), "items", &Array::new().into())?;
                    toast_error("No pude cargar snippets: ", e)?;
                }
            }
            let sessions = async {
                let state = wait(exported("api", &["/state?live=1".into()])).await?;
                let state = if truthy(&state) {
                    state
                } else {
                    Array::new().into()
                };
                call(
                    &state,
                    "map",
                    &[function(|a| {
                        let session = a.get(0);
                        let out = object();
                        let key = get(&session, "session");
                        set(&out, "session", &key)?;
                        let project = get(&session, "project");
                        set(
                            &out,
                            "project",
                            &if truthy(&project) { project } else { key },
                        )?;
                        Ok(out)
                    })],
                )
            }
            .await
            .unwrap_or_else(|_| Array::new().into());
            set(&js_sys::global(), "snipSessions", &sessions)?;
            set(&snips(), "selectedId", &JsValue::NULL)?;
            render_list()?;
            render_pane()?;
            Ok(JsValue::UNDEFINED)
        }))
    }
    fn close() -> Result<(), JsValue> {
        let dlg = global("snipDlg");
        if truthy(&get(&dlg, "open")) {
            call(&dlg, "close", &[])?;
        }
        Ok(())
    }
    pub fn mount() -> Result<(), JsValue> {
        set(
            &js_sys::global(),
            "snipState",
            &from_json(&json!({"items":[],"selectedId":null,"query":""}))?,
        )?;
        set(&js_sys::global(), "snipSessions", &Array::new().into())?;
        publish("pickDefaultSession", |a| {
            Ok(default_session(a.get(0), a.get(1)))
        })?;
        publish("filterSnippets", |a| filter(a.get(0), a.get(1)))?;
        publish("renderSnipList", |_| {
            render_list()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("renderSnipPane", |_| {
            render_pane()?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("openSnipEditor", |a| {
            editor(a.get(0))?;
            Ok(JsValue::UNDEFINED)
        })?;
        publish("openSnippets", |_| {
            Ok(open().unwrap_or_else(|e| Promise::reject(&e).into()))
        })?;
        publish("closeSnippets", |_| {
            close()?;
            Ok(JsValue::UNDEFINED)
        })
    }
    pub fn attach() -> Result<(), JsValue> {
        let dlg = id("dlg-snippets");
        set(&js_sys::global(), "snipDlg", &dlg)?;
        listen(&id("btn-snippets"), "click", global("openSnippets"));
        listen(
            &query(&dlg, "[data-close-snippets]"),
            "click",
            global("closeSnippets"),
        );
        listen(
            &dlg.clone(),
            "click",
            function(move |a| {
                if js_sys::Object::is(&get(&a.get(0), "target"), &dlg) {
                    close()?;
                }
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &id("snip-q"),
            "input",
            function(|a| {
                set(&snips(), "query", &get(&get(&a.get(0), "target"), "value"))?;
                render_list()?;
                Ok(JsValue::UNDEFINED)
            }),
        );
        listen(
            &id("snip-new"),
            "click",
            function(|_| {
                editor(JsValue::NULL)?;
                Ok(JsValue::UNDEFINED)
            }),
        );
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
export function snippet_fixture(){
 globalThis.snipCalls=[];globalThis.snipToasts=[];globalThis.copied=[];globalThis.allowDelete=true;
 const node=(tag='div',id='')=>{const e={tagName:tag.toUpperCase(),id,dataset:{},style:{},textContent:'',placeholder:'',events:{},children:[],classList:new Set(),addEventListener(n,f){(this.events[n]??=[]).push(f)},appendChild(x){this.children.push(x);x.parent=this;return x},focus(){this.focused=true},showModal(){this.open=true},close(){this.open=false},querySelector(s){return s==='[data-close-snippets]'?closeButton:null}};e.classList.contains=e.classList.has;e.classList.toggle=(c,on)=>on?e.classList.add(c):e.classList.delete(c);Object.defineProperty(e,'innerHTML',{get(){return this._html??''},set(v){this._html=v;this.children=[]}});Object.defineProperty(e,'value',{get(){if(this._value!==undefined)return this._value;if(this.tagName==='SELECT')return (this.children.find(x=>x.selected)??this.children.find(x=>!x.disabled))?.value??'';return ''},set(v){this._value=String(v)}});return e};
 globalThis.snipNode=node;globalThis.snipNodes={};for(const id of ['dlg-snippets','snip-q','snip-items','snip-pane','btn-snippets','snip-new'])snipNodes[id]=node(id==='dlg-snippets'?'dialog':'div',id);const closeButton=node('button');
 const find=(root,id)=>{if(root.id===id)return root;for(const c of root.children){const got=find(c,id);if(got)return got}return null};
 globalThis.document={getElementById:k=>snipNodes[k]??Object.values(snipNodes).map(r=>find(r,k)).find(Boolean)??null,createElement:t=>node(t)};
 globalThis.toast=(...a)=>snipToasts.push(a);globalThis.confirm=()=>allowDelete;
 const store=new Map([['snippet-last-session','second']]);globalThis.localStorage={getItem:k=>store.get(k)??null,setItem:(k,v)=>store.set(k,String(v))};
 Object.defineProperty(globalThis,'navigator',{configurable:true,value:{clipboard:{writeText:t=>{copied.push(t);return Promise.resolve()}}}});
 globalThis.library=[{id:'one',name:'<script> name',body:'echo 😀\n\ud800',tags:['prod','alpha'],updated_at:5},{id:'two',name:'docker ps',body:'docker ps',tags:['docker'],updated_at:3}];globalThis.sessions=[{session:'first',project:'First'},{session:'second',project:'Second'}];globalThis.snipFailure='';
 globalThis.api=(path,body)=>{snipCalls.push({path,body});if(snipFailure===path)return Promise.reject(Error('fixture failure'));if(path==='/snippets'&&body===undefined)return Promise.resolve(library);if(path==='/state')return Promise.resolve(sessions);if(path==='/snippets'||path==='/snippets/update')return Promise.resolve({item:{id:body.id??'new',...body,updated_at:9}});return Promise.resolve({ok:true});};
}
export async function snippet_contracts(source){
 const eq=(a,b,m)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(m+': '+JSON.stringify(a)+' != '+JSON.stringify(b))};const ok=(x,m)=>{if(!x)throw Error(m)};const drain=async()=>{for(let i=0;i<24;i++)await Promise.resolve()};
 const cut=(a,b)=>source.slice(source.indexOf(a),source.indexOf(b,source.indexOf(a)));
 const original=vm.runInNewContext(cut('function pickDefaultSession','function renderSnipList')+'\n({pickDefaultSession,filterSnippets})');
 const data=[...library,{id:'three',name:'newer alpha',body:'old',tags:[],updated_at:8},{id:'four',name:'tag only',body:'ALPHA',tags:['alpha'],updated_at:2},{id:'five',name:'body only',body:'alpha',tags:[],updated_at:20}];
 for(const q of ['', ' alpha ','DOCKER','😀','\ud800','no match'])eq(filterSnippets(data,q).map(x=>x.id),original.filterSnippets(data,q).map(x=>x.id),'original ranking '+JSON.stringify(q));
 for(const list of [[],sessions,[{session:'first'}]])for(const last of ['', 'second','gone'])eq(pickDefaultSession(list,last),original.pickDefaultSession(list,last),'original default target');
 ok(filterSnippets(data,'')[0]===data[4],'filter retains object identity');
 await openSnippets();ok(snipDlg===snipNodes['dlg-snippets'],'same native dialog');ok(snipDlg.open,'showModal');ok(snipNodes['snip-q'].focused,'query focus');eq(snipCalls.slice(0,2),[{path:'/snippets'},{path:'/state'}],'open route order');eq(snipState.selectedId,'one','auto selection');eq(snipNodes['snip-items'].children[0].children[0].textContent,'<script> name','list text safety');eq(snipNodes['snip-pane'].children[0].textContent,'<script> name','pane text safety');eq(snipNodes['snip-pane'].children.find(x=>x.tagName==='PRE').textContent,library[0].body,'raw Unicode body');
 let actions=snipNodes['snip-pane'].children.slice(-1)[0],sel=actions.children.find(x=>x.tagName==='SELECT');eq(sel.value,'second','saved destination');eq(sel.title,'Sesión destino: el comando aparecerá en el prompt de esa pestaña, esperando tu Enter','explicit no Enter title');
 const click=async(button)=>{await Promise.all(button.events.click.map(f=>f({target:button})));await drain()};
 await click(actions.children.find(x=>x.textContent==='Send'));eq(snipCalls.slice(-1)[0],{path:'/paste',body:{session:'second',text:library[0].body}},'paste without enter');ok(!('enter' in snipCalls.slice(-1)[0].body),'no enter field');eq(localStorage.getItem('snippet-last-session'),'second','saved successful destination');
 await click(actions.children.find(x=>x.textContent==='Copy'));eq(copied.slice(-1)[0],library[0].body,'clipboard exact UTF16');snipFailure='/paste';sel.value='first';await click(actions.children.find(x=>x.textContent==='Send'));eq(localStorage.getItem('snippet-last-session'),'second','failed paste does not persist target');eq(snipToasts.slice(-1)[0],['fixture failure',true],'paste error');snipFailure='';const oldCopy=navigator.clipboard.writeText;navigator.clipboard.writeText=()=>Promise.reject(Error('denied'));await click(actions.children.find(x=>x.textContent==='Copy'));eq(snipToasts.slice(-1)[0],['No pude copiar: denied',true],'clipboard error prefix');navigator.clipboard.writeText=oldCopy;
 await click(actions.children.find(x=>x.textContent==='Edit'));let pane=snipNodes['snip-pane'];eq(pane.children.map(x=>x.tagName),['LABEL','INPUT','LABEL','INPUT','LABEL','TEXTAREA','DIV'],'editor complete DOM');ok(pane.children[1].focused,'name focus');pane.children[1].value='renamed';pane.children[3].value='one, two, , three';pane.children[5].value='  unchanged newline\n';snipFailure='/snippets/update';await click(pane.children[6].children[0]);eq(snipState.items.find(x=>x.id==='one').name,'<script> name','failed update retains old item');snipFailure='';await click(pane.children[6].children[0]);eq(snipCalls.slice(-1)[0],{path:'/snippets/update',body:{name:'renamed',body:'  unchanged newline\n',tags:['one','two','three'],id:'one'}},'update values');eq(snipState.items.find(x=>x.id==='one').name,'renamed','update item replaced');
 openSnipEditor(null);pane=snipNodes['snip-pane'];const before=snipCalls.length;await click(pane.children[6].children[0]);eq(snipCalls.length,before,'empty name no request');eq(snipToasts.slice(-1)[0],['Nombre vacío',true],'name validation');pane.children[1].value='fresh';pane.children[5].value=' \n ';await click(pane.children[6].children[0]);eq(snipToasts.slice(-1)[0],['Body vacío',true],'body validation');pane.children[5].value='pwd';await click(pane.children[6].children[0]);eq(snipCalls.slice(-1)[0],{path:'/snippets',body:{name:'fresh',body:'pwd',tags:[]}},'create route');eq(snipState.selectedId,'new','created selection');
 actions=snipNodes['snip-pane'].children.slice(-1)[0];const deletion=actions.children.find(x=>x.textContent==='Delete');allowDelete=false;const count=snipCalls.length;await click(deletion);eq(snipCalls.length,count,'cancel deletion');allowDelete=true;await click(deletion);ok(!snipState.items.some(x=>x.id==='new'),'successful delete');eq(snipCalls.slice(-1)[0],{path:'/snippets/delete',body:{id:'new'}},'delete route');
 snipNodes['snip-q'].value='docker';snipNodes['snip-q'].events.input[0]({target:snipNodes['snip-q']});eq(snipNodes['snip-items'].children.length,1,'input filter');eq(snipState.query,'docker','query state');
 snipSessions=[];renderSnipPane();actions=snipNodes['snip-pane'].children.slice(-1)[0];await click(actions.children.find(x=>x.textContent==='Send'));eq(snipToasts.slice(-1)[0],['No hay sesión destino',true],'no destination');
 closeSnippets();ok(!snipDlg.open,'close native dialog');snipDlg.open=true;snipDlg.events.click[0]({target:snipDlg});ok(!snipDlg.open,'overlay closes');
 snipFailure='/snippets';await openSnippets();eq(snipState.items,[],'list load failure clears');eq(snipToasts.slice(-1)[0],['No pude cargar snippets: fixture failure',true],'list load error prefix');ok(snipNodes['snip-pane'].innerHTML.includes('snip-empty'),'empty pane');snipFailure='/state';await openSnippets();eq(snipSessions,[],'state load failure clears destinations');
}
"#)]
    extern "C" {
        fn snippet_fixture();
        #[wasm_bindgen(catch)]
        async fn snippet_contracts(source: &str) -> Result<JsValue, JsValue>;
    }
    #[wasm_bindgen_test(async)]
    async fn snippets_preserve_original_search_and_complete_crud_dom_callbacks() {
        snippet_fixture();
        mount().unwrap();
        attach().unwrap();
        snippet_contracts(include_str!("../../../../dash/index.html"))
            .await
            .unwrap();
    }
}
