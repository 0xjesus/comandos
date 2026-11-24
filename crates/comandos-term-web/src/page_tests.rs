//! Browser regression uses a private fake transport, never a real session.
use super::*;
use wasm_bindgen_test::wasm_bindgen_test;
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
#[wasm_bindgen(inline_js = r#"
let savedSocket, savedUrl, savedFetch, fixture;
export function setupPage() {
  savedSocket=window.WebSocket; savedUrl=location.href;savedFetch=window.fetch;
  window.__paneRequests=[];window.__readyRequests=[];
  window.fetch=(url,opts)=>{if(url==='/web/ready'){window.__readyRequests.push(JSON.parse(opts.body));return Promise.resolve(new Response('{}',{status:200}));}if(url!=='/terminal-panes')return savedFetch(url,opts);const body=JSON.parse(opts.body);window.__paneRequests.push(body);return Promise.resolve(new Response(JSON.stringify({ok:true,panes:[{id:'%42',identity:'private-identity'}],clientKeys:body.action==='select'?'\x1b[Z':undefined}),{status:200,headers:{'Content-Type':'application/json'}}));};
  window.__pageSockets=[];
  window.WebSocket=class extends EventTarget {
    constructor(url,protocols) {super();this.url=url;this.offered=[...protocols];this.protocol='';this.readyState=0;this.sent=[];window.__pageSockets.push(this);}
    send(data) {if(this.readyState!==1)throw Error('closed');this.sent.push(typeof data==='string'?data:Array.from(data));}
    close(code=1000,reason='') {if(this.readyState===3)return;this.readyState=3;queueMicrotask(()=>this.dispatchEvent(new CloseEvent('close',{code,reason})));}
  };
  history.replaceState(null,'','?auth=private-token&arg=private-session&theme=dia&btn=pixel&debug=1&ws='+encodeURIComponent('ws://example.invalid/ws?keep=1'));
  const root=document.createElement('div');root.id='page-fixture';
  root.innerHTML='<div id="term-shell"><div id="term" style="width:640px;height:240px"></div><div id="term-toolbar"><button data-action="mode"></button></div></div><div id="err"></div><div id="dbg"></div>';
  document.body.appendChild(root);
}
export function restorePage() {window.WebSocket=savedSocket;window.fetch=savedFetch;delete window.__paneRequests;history.replaceState(null,'',savedUrl);document.querySelector('#page-fixture')?.remove();delete window.__pageSockets;}
export function openSocket(i,protocol) {const s=window.__pageSockets[i];s.protocol=protocol;s.readyState=1;s.dispatchEvent(new Event('open'));}
export function outputSocket(i,data) {window.__pageSockets[i].dispatchEvent(new MessageEvent('message',{data}));}
export function closeSocket(i) {window.__pageSockets[i].close(1006);}
export function socketInfo(i) {const s=window.__pageSockets[i];return JSON.stringify({url:s.url,offered:s.offered,sent:s.sent});}
export function detachFixture() {fixture=document.querySelector('#page-fixture');fixture.remove();Object.defineProperty(document,'readyState',{configurable:true,value:'loading'});}
export function attachFixture() {delete document.readyState;document.body.appendChild(fixture);document.dispatchEvent(new Event('DOMContentLoaded'));}
export function readyRequests() {return JSON.stringify(window.__readyRequests);}
export function paneRequests() {return JSON.stringify(window.__paneRequests);}
export function socketCount() {return window.__pageSockets.length;}
export function parentTheme(origin,source,theme) {window.dispatchEvent(new MessageEvent('message',{origin:origin?location.origin:'https://hostile.invalid',source:source?window:null,data:{source:'comandos',type:'theme',theme}}));}
"#)]
extern "C" {
    #[wasm_bindgen(js_name=setupPage)]
    fn setup();
    #[wasm_bindgen(js_name=restorePage)]
    fn restore();
    #[wasm_bindgen(js_name=openSocket)]
    fn open_socket(i: u32, protocol: &str);
    #[wasm_bindgen(js_name=outputSocket)]
    fn output_socket(i: u32, data: &JsValue);
    #[wasm_bindgen(js_name=closeSocket)]
    fn close_socket(i: u32);
    #[wasm_bindgen(js_name=socketInfo)]
    fn socket_info(i: u32) -> String;
    #[wasm_bindgen(js_name=detachFixture)]
    fn detach_fixture();
    #[wasm_bindgen(js_name=attachFixture)]
    fn attach_fixture();
    #[wasm_bindgen(js_name=readyRequests)]
    fn ready_requests() -> String;
    #[wasm_bindgen(js_name=paneRequests)]
    fn pane_requests() -> String;
    #[wasm_bindgen(js_name=socketCount)]
    fn socket_count() -> u32;
    #[wasm_bindgen(js_name=parentTheme)]
    fn parent_theme(origin: bool, source: bool, theme: &str);
}
fn info(i: u32) -> Value {
    serde_json::from_str(&socket_info(i)).unwrap()
}
struct Fixture;
impl Drop for Fixture {
    fn drop(&mut self) {
        PAGE.with(|slot| {
            if let Some(page) = slot.borrow_mut().take() {
                dispose(&page);
            }
        });
        restore();
    }
}
#[wasm_bindgen_test(async)]
async fn page_transport_parent_ack_and_disposal() {
    setup();
    let _fixture = Fixture;
    boot_term("private-ticket").unwrap();
    let ready: Value = serde_json::from_str(&ready_requests()).unwrap();
    assert_eq!(
        ready[0],
        json!({"k":"private-ticket","mounted":["term"],"failed":[]})
    );
    let page = PAGE.with(|s| s.borrow().clone().unwrap());
    assert_eq!(socket_count(), 1);
    assert_eq!(info(0)["offered"], json!(["comandos.term.v1", "tty"]));
    assert!(
        info(0)["url"]
            .as_str()
            .unwrap()
            .contains("keep=1&arg=private-token&arg=private-session")
    );
    assert_eq!(
        web_sys::Url::new(info(0)["url"].as_str().unwrap())
            .unwrap()
            .search_params()
            .get("token"),
        Some("private-token".into())
    );
    parent_theme(false, true, "noche");
    assert_eq!(page.borrow().theme, "dia");
    parent_theme(true, false, "noche");
    assert_eq!(page.borrow().theme, "dia");
    parent_theme(true, true, "noche");
    assert_eq!(page.borrow().theme, "noche");
    open_socket(0, "comandos.term.v1");
    let init: Value = serde_json::from_str(info(0)["sent"][0].as_str().unwrap()).unwrap();
    assert_eq!(init["v"], 1);
    assert_eq!(init["session"], "private-session");
    assert!(init.get("AuthToken").is_none());
    let promise = send_with_ack(b"draft", 1000);
    assert_eq!(page.borrow().ack_callbacks.len(), 1);
    output_socket(0, &"0response".into());
    assert_eq!(JsFuture::from(promise).await.unwrap(), JsValue::TRUE);
    let expired = send_with_ack(b"timeout", 1);
    delay(20).await;
    assert_eq!(JsFuture::from(expired).await.unwrap(), JsValue::FALSE);
    output_socket(0, &"0\x1b[6n".into());
    assert!(info(0)["sent"].as_array().unwrap().iter().any(|v| {
        v.as_array()
            .is_some_and(|a| a.first() == Some(&json!(48)) && a.get(1) == Some(&json!(27)))
    }));
    handle(&page, Command::Ctrl);
    let callback = page
        .borrow()
        .callbacks
        .first()
        .unwrap()
        .as_ref()
        .unchecked_ref::<Function>()
        .clone();
    callback
        .call1(&JsValue::NULL, &Uint8Array::from(&b"["[..]))
        .unwrap();
    assert!(!page.borrow().ctrl);
    assert_eq!(
        info(0)["sent"].as_array().unwrap().last(),
        Some(&json!([48, 27]))
    );
    handle(&page, Command::Ctrl);
    callback
        .call1(&JsValue::NULL, &Uint8Array::from(&b"1"[..]))
        .unwrap();
    assert!(page.borrow().ctrl);
    handle(&page, Command::Paste("plain".into()));
    assert!(page.borrow().ctrl);
    set_ctrl(&page, false);
    close_socket(0);
    delay(1).await;
    handle(&page, Command::Paste("must-not-queue".into()));
    let bytes = Uint8Array::from(&[255_u8, 128, 0][..]);
    let callback = page
        .borrow()
        .callbacks
        .first()
        .unwrap()
        .as_ref()
        .unchecked_ref::<Function>()
        .clone();
    callback.call1(&JsValue::NULL, &bytes).unwrap();
    delay(300).await;
    assert_eq!(socket_count(), 2);
    open_socket(1, "tty");
    let init: Value = serde_json::from_str(info(1)["sent"][0].as_str().unwrap()).unwrap();
    assert_eq!(init["AuthToken"], "");
    assert!(init.get("session").is_none());
    assert_eq!(info(1)["sent"][1], json!([48, 255, 128, 0]));
    select_pane(&page, "%42".into());
    delay(30).await;
    // tmux is still attaching (nothing drawn): a focus key now would be lost.
    assert_eq!(pane_requests(), "[]");
    output_socket(1, &JsValue::from_str("0$ "));
    delay(600).await;
    let requests: Value = serde_json::from_str(&pane_requests()).unwrap();
    assert_eq!(requests[0]["action"], "list");
    assert_eq!(requests[1]["scope"], "client");
    assert_eq!(requests[1]["identity"], "private-identity");
    let t = term(&page).unwrap();
    t.borrow_mut().dispatch(|i| {
        i.select
            .start(comandos_term::select::SelectMode::Simple, (0, 0));
        i.select.extend((0, 3));
    });
    assert!(t.borrow().has_selection());
    let original_size = t.borrow().with(|i| i.size);
    element("term")
        .unwrap()
        .style()
        .set_property("width", "580px")
        .unwrap();
    fit(&page);
    assert_eq!(t.borrow().with(|i| i.size), original_size);
    assert_eq!(socket_count(), 2);
    t.borrow_mut().clear_selection();
    drop(t);
    delay(200).await;
    assert_eq!(socket_count(), 3);
    assert!(!page.borrow().down);
    open_socket(2, "comandos.term.v1");
    element("term")
        .unwrap()
        .style()
        .set_property("height", "300px")
        .unwrap();
    fit(&page);
    let frames = info(2)["sent"].as_array().unwrap().len();
    fit(&page);
    assert_eq!(info(2)["sent"].as_array().unwrap().len(), frames);
    assert!(
        info(2)["sent"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with('1')
    );
    let current_ack = send_with_ack(b"current", 1000);
    output_socket(1, &"0stale".into());
    assert_eq!(page.borrow().ack_callbacks.len(), 1);
    output_socket(2, &"0current".into());
    assert_eq!(JsFuture::from(current_ack).await.unwrap(), JsValue::TRUE);
    let closed_ack = send_with_ack(b"session", 1000);
    switch_session(&page, "next-private");
    assert_eq!(JsFuture::from(closed_ack).await.unwrap(), JsValue::FALSE);
    delay(20).await;
    assert_eq!(socket_count(), 4);
    open_socket(3, "comandos.term.v1");
    let init: Value = serde_json::from_str(info(3)["sent"][0].as_str().unwrap()).unwrap();
    assert_eq!(init["session"], "next-private");
    output_socket(3, &"2{\"fontSize\":11,\"fontFamily\":\"monospace\"}".into());
    assert_eq!(
        term(&page).unwrap().borrow().with(|i| i.opts.font_size),
        Some(11.0)
    );
    output_socket(3, &"2{\"fontFamily\":\"monospace\"}".into());
    assert_eq!(
        term(&page).unwrap().borrow().with(|i| i.opts.font_size),
        Some(11.0)
    );
    // reset discards parser replies, OSC state and selection together.
    let t = term(&page).unwrap();
    t.borrow_mut().write(b"\x1b]2;old-title\x07\x1b[6n");
    t.borrow_mut().reset();
    assert!(t.borrow_mut().take_replies().is_empty());
    assert!(t.borrow_mut().take_title().is_none());
    drop(t);
    let count = socket_count();
    dispose(&page);
    delay(500).await;
    assert_eq!(socket_count(), count);
    assert!(page.borrow().term.is_none());
    assert!(page.borrow().ack_callbacks.is_empty());
    // A head gate receives its ticket before parsing reaches the terminal DOM.
    detach_fixture();
    boot_term("head-ticket").unwrap();
    assert_eq!(socket_count(), count);
    assert_eq!(
        serde_json::from_str::<Value>(&ready_requests()).unwrap()[1]["k"],
        "head-ticket"
    );
    attach_fixture();
    assert_eq!(socket_count(), count + 1);
    let attached = PAGE.with(|s| s.borrow().clone().unwrap());
    assert!(attached.borrow().term.is_some());
    open_socket(count, "comandos.term.v1");
    let rendered = term(&attached).unwrap();
    rendered
        .borrow_mut()
        .write(&b"preserved during drag\r\n".repeat(60));
    let history = rendered.borrow().with(|i| i.engine.history_len());
    assert!(history.is_some_and(|n| n > 0));
    let pending = send_with_ack(b"draft during drag", 2000);
    for width in [600, 560, 520, 480] {
        element("term")
            .unwrap()
            .style()
            .set_property("width", &format!("{width}px"))
            .unwrap();
        schedule_fit(&attached);
        delay(80).await;
        assert_eq!(
            socket_count(),
            count + 1,
            "continuous width changes must not reconnect"
        );
        assert_eq!(
            rendered.borrow().with(|i| i.engine.history_len()),
            history,
            "drag must preserve history until quiet"
        );
        assert_eq!(
            attached.borrow().ack_callbacks.len(),
            1,
            "drag must leave draft acknowledgement pending"
        );
    }
    delay(60).await; // 140 ms since the final width change, still below 180.
    assert_eq!(socket_count(), count + 1);
    delay(90).await;
    assert_eq!(
        socket_count(),
        count + 2,
        "one reconnection follows 180 ms of quiet"
    );
    assert_eq!(JsFuture::from(pending).await.unwrap(), JsValue::FALSE);
    open_socket(count + 1, "comandos.term.v1");
    let frames = info(count + 1)["sent"].as_array().unwrap().len();
    let rows = rendered.borrow().with(|i| i.size.rows);
    for height in [340, 380, 420] {
        element("term")
            .unwrap()
            .style()
            .set_property("height", &format!("{height}px"))
            .unwrap();
        schedule_fit(&attached);
        delay(80).await;
        assert_eq!(
            info(count + 1)["sent"].as_array().unwrap().len(),
            frames,
            "height events reset their 120 ms debounce"
        );
        assert_eq!(rendered.borrow().with(|i| i.size.rows), rows);
    }
    delay(20).await;
    assert_eq!(info(count + 1)["sent"].as_array().unwrap().len(), frames);
    delay(80).await;
    assert_eq!(
        info(count + 1)["sent"].as_array().unwrap().len(),
        frames + 1
    );
    assert_eq!(socket_count(), count + 2);
    drop(rendered);
    dispose(&attached);
}
