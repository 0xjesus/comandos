"""Compile exact owned T13 arms/signal callbacks with inert notebook collaborators.

Run manually with a scratch output directory, PKG_CONFIG_PATH/CARGO_TARGET_DIR and
SDK LD_LIBRARY_PATH. Never initialise GTK or launch a browser/backend/tmux.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

checkout = Path(__file__).resolve().parents[4]
out = Path(sys.argv[1]).resolve()
assert out != checkout and checkout not in out.parents
out.mkdir(parents=True, exist_ok=True)
(out / 'src').mkdir(exist_ok=True)
app_path = checkout / 'crates/comandos-app/src/ui/app.rs'
t13_path = checkout / 'crates/comandos-app/src/ui/app_t13.rs'
app, t13 = app_path.read_text(), t13_path.read_text()

def block(source, start):
    at = source.index(start)
    brace = source.index('{', at)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[at:end]

methods = block(t13, '    pub(in crate::ui) fn install_app_commands').replace('pub(in crate::ui) ', '')
for name in ['active_notebook', 'navigation_index', 'remember_navigation_page', 'remember_current_navigation_page', 'forget_navigation_page']:
    if f'fn {name}(' in t13:
        methods += '\n' + block(t13, f'fn {name}(').replace('pub(super) ', '')
arms = '\n'.join(block(t13, f'            "{name}"') for name in ['next_tab', 'mru_toggle', 'open_xterm_tab'])
vis_at = t13.index('            "terminals_visible" =>')
arms += '\n' + t13[vis_at:t13.index('            "reload_dashboard"', vis_at)]
methods += '\n' + block(t13, '    pub(super) fn shutdown_protocol').replace('pub(super) ', '')
methods += '\n' + block(app, '    fn writable(')
run_body = block(t13, '    fn run_app_command(')
gate = run_body[run_body.index('{') + 1:run_body.index('        if let Some(consumer)')]
methods += '\nfn run_app_command(self: &Rc<Self>, name:&str, args:&Value)->Result<(),CommandError>{' + gate + 'match name {\n' + arms + '\n_=>return Err(CommandError::Unknown(name.into()))};Ok(())}'
# Helpers remain the exact source bodies; only GTK/App collaborators are inert.
preamble = r'''
use comandos_app::ui::app_commands::{self as commands, CommandError};
use serde_json::{Value,json};
use std::{cell::{Cell,RefCell},rc::Rc,sync::{Arc,atomic::{AtomicBool,Ordering}},collections::BTreeMap};
mod gtk { pub type Widget=String; pub type Notebook=super::Notebook; #[derive(Clone,Copy,PartialEq)] pub enum PropagationPhase { None } }
mod glib { macro_rules! closure_local { ($cb:expr) => { $cb }; } pub(crate) use closure_local; }
type Signal=usize;
type Switch=Rc<dyn Fn(&Notebook,&String,u32)>;
#[derive(Clone)] struct Notebook(Rc<NotebookState>);
struct NotebookState{pages:RefCell<Vec<String>>,current:Cell<u32>,visible:Cell<bool>,signals:RefCell<BTreeMap<Signal,Switch>>,next:Cell<usize>}
impl PartialEq for Notebook{fn eq(&self,other:&Self)->bool{Rc::ptr_eq(&self.0,&other.0)}}
impl Notebook{
 fn new(pages:&[&str])->Self{Self(Rc::new(NotebookState{pages:RefCell::new(pages.iter().map(|s|s.to_string()).collect()),current:Cell::new(0),visible:Cell::new(true),signals:RefCell::new(BTreeMap::new()),next:Cell::new(0)}))}
 fn n_pages(&self)->u32{self.0.pages.borrow().len() as u32}
 fn nth_page(&self,index:Option<u32>)->Option<String>{self.0.pages.borrow().get(index? as usize).cloned()}
 fn current_page(&self)->Option<u32>{(self.n_pages()>0).then(||self.0.current.get())}
 fn set_current_page(&self,p:Option<u32>){if let Some(p)=p{self.0.current.set(p);let page=self.0.pages.borrow().get(p as usize).unwrap().clone();let callbacks:Vec<_>=self.0.signals.borrow().values().cloned().collect();for cb in callbacks{cb(self,&page,p)}}}
 fn page_num(&self,page:&String)->Option<u32>{self.0.pages.borrow().iter().position(|p|p==page).map(|p|p as u32)}
 fn set_visible(&self,b:bool){self.0.visible.set(b)}
 fn connect_switch_page(&self,cb:impl Fn(&Notebook,&String,u32)+'static)->Signal{let id=self.0.next.get();self.0.next.set(id+1);self.0.signals.borrow_mut().insert(id,Rc::new(cb));id}
 fn connect_closure(&self,name:&str,after:bool,cb:impl Fn(Notebook,String,u32)+'static)->Signal{assert_eq!(name,"switch-page");assert!(after);self.connect_switch_page(move |n,p,i|cb(n.clone(),p.clone(),i))}
 fn upcast(&self)->Self{self.clone()}
 fn disconnect(&self,id:Signal){assert!(self.0.signals.borrow_mut().remove(&id).is_some());}
}
struct Workspace{root:Notebook,applying:Cell<bool>,keys:RefCell<BTreeMap<String,u32>>,focused:RefCell<Option<String>>}
impl Workspace{
 fn widget(&self)->&Notebook{&self.root}
 fn focus_page(&self,page:&String){let i=self.root.page_num(page).unwrap();let keep=self.focused.borrow().clone().filter(|k|self.page_index(k)==Some(i));*self.focused.borrow_mut()=keep.or_else(||self.page_key(page));}
 fn is_applying(&self)->bool{self.applying.get()}
 fn page_index(&self,key:&str)->Option<u32>{self.keys.borrow().get(key).copied()}
 fn page_key(&self,page:&String)->Option<String>{let index=self.root.page_num(page)?;self.keys.borrow().iter().find(|(_,i)|**i==index).map(|(k,_)|k.clone())}
}
struct Strip{root:Notebook,keys:BTreeMap<String,String>}
impl Strip{fn page_key(&self,page:&String)->Option<String>{self.keys.iter().find(|(_,p)|*p==page).map(|(k,_)|k.clone())}fn page_index(&self,key:&str)->Option<u32>{self.root.page_num(self.keys.get(key)?)} }
struct Config{writes:Cell<bool>}impl Config{fn writes_allowed(&self)->bool{self.writes.get()}}
struct Restore{is_ready:Cell<bool>}impl Restore{fn ready(&self)->bool{self.is_ready.get()}}
struct Cancellable(Cell<bool>);impl Cancellable{fn cancel(&self){self.0.set(true)}}
struct Controller;impl Controller{fn reset(&self){}fn set_propagation_phase(&self,phase:gtk::PropagationPhase){assert!(phase==gtk::PropagationPhase::None)}}
struct App{cfg:Config,startup_valid:Cell<bool>,restore:RefCell<Restore>,bridge_queue:RefCell<comandos_app::ui::bridge::BridgeQueue>,protocol_cancellable:Cancellable,interaction_controllers:RefCell<Vec<Controller>>,notebook:Notebook,workspace:Workspace,workspace_doc:RefCell<Value>,strip:RefCell<Strip>,mru_pages:RefCell<Vec<String>>,closed:Arc<AtomicBool>,handlers:commands::Registry,protocol_signals:RefCell<Vec<(Notebook,Signal)>>,events:RefCell<Vec<Value>>}
impl App{
 fn install_handler(&self,name:&'static str,cb:commands::Handler){self.handlers.install(name,cb)}
 fn add_tab(&self,key:&str,label:&str,_:bool,_:Option<()>){self.events.borrow_mut().push(json!(["add_tab",key,label]));}
 fn select(&self,key:&str){self.events.borrow_mut().push(json!(["select",key]));}
 fn persist(&self){self.events.borrow_mut().push(json!(["persist"]));}
 fn new(workspace:bool)->Rc<Self>{let legacy=Notebook::new(if workspace{&[]}else{&["a-old","b-old"]});legacy.set_visible(!workspace);Rc::new(Self{cfg:Config{writes:Cell::new(true)},startup_valid:Cell::new(true),restore:RefCell::new(Restore{is_ready:Cell::new(true)}),bridge_queue:RefCell::new(comandos_app::ui::bridge::BridgeQueue::default()),protocol_cancellable:Cancellable(Cell::new(false)),interaction_controllers:RefCell::new(vec![Controller]),notebook:legacy.clone(),workspace:Workspace{root:Notebook::new(if workspace{&["a-new","b-new"]}else{&[]}),applying:Cell::new(false),focused:RefCell::new(Some("a".into())),keys:RefCell::new([("a".into(),0),("b".into(),1)].into())},workspace_doc:RefCell::new(if workspace{json!({"groups":[]})}else{Value::Null}),strip:RefCell::new(Strip{root:legacy,keys:[("a".into(),"a-old".into()),("b".into(),"b-old".into())].into()}),mru_pages:RefCell::new(vec![]),closed:Arc::new(AtomicBool::new(false)),handlers:commands::Registry::default(),protocol_signals:RefCell::new(vec![]),events:RefCell::new(vec![])})}
'''
main = r'''
}
fn main(){for guard in 0..4{let app=App::new(true);app.install_app_commands();match guard{0=>app.cfg.writes.set(false),1=>app.startup_valid.set(false),2=>app.restore.borrow().is_ready.set(false),_=>app.closed.store(true,Ordering::Release)};for name in ["next_tab","prev_tab","mru_toggle","terminals_visible","open_xterm_tab"]{assert!(app.handlers.invoke(name,&json!({"on":false})).is_err());}assert_eq!(app.workspace.root.current_page(),Some(0));assert!(app.workspace.root.0.visible.get());assert!(app.events.borrow().is_empty());}let app=App::new(false);app.install_app_commands();{let _held=app.strip.borrow_mut();app.notebook.set_current_page(Some(1));}app.remember_current_navigation_page();assert_eq!(*app.mru_pages.borrow(),vec!["a".to_string(),"b".to_string()]);let app=App::new(true);app.install_app_commands();app.workspace.root.set_current_page(Some(1));app.workspace.keys.borrow_mut().remove("a");app.forget_navigation_page("a");assert_eq!(*app.mru_pages.borrow(),vec!["b".to_string()]);let mut results=vec![];
for ws in [false,true]{let app=App::new(ws);app.install_app_commands();let active=if ws{app.workspace.widget()}else{&app.notebook};active.set_current_page(Some(1));active.set_current_page(Some(0));
let tracking=app.mru_pages.borrow().len()==2;
let mut nav=vec![];for (name,want) in [("next_tab",1),("prev_tab",0),("mru_toggle",1),("mru_toggle",0)]{let parsed=commands::parse_command(&json!({"command":name,"args":true}));let success=parsed.and_then(|c|app.handlers.dispatch(&c)).is_ok();nav.push(json!({"name":name,"success":success,"current":active.current_page(),"want":want}));if ws{assert_eq!(app.workspace.focused.borrow().as_deref(),Some(if want==0{"a"}else{"b"}));}}
for on in [false,true]{app.handlers.invoke("terminals_visible",&json!({"on":on})).unwrap();assert_eq!(active.0.visible.get(),on);}
results.push(json!({"workspace":ws,"tracking":tracking,"navigation":nav}));}
let app=App::new(false);app.install_app_commands();app.notebook.set_current_page(Some(1));app.notebook.set_current_page(Some(0));
app.workspace.applying.set(true);app.notebook.0.pages.borrow_mut().clear();app.workspace.root.0.pages.borrow_mut().extend(["a-reparented".into(),"b-reparented".into()]);app.workspace.root.set_current_page(Some(0));*app.workspace_doc.borrow_mut()=json!({"groups":[]});app.workspace.applying.set(false);
// These calls are actual production apply/refresh hooks checked below by the generator.
'''
if 'fn remember_current_navigation_page(' in t13:
    main += 'app.remember_current_navigation_page();\n'
main += r'''
let mut reparent=vec![];for want in [1,0,1,0]{app.handlers.invoke("mru_toggle",&json!({})).unwrap();reparent.push(json!([app.workspace.root.current_page(),want]));}
app.workspace.applying.set(true);app.workspace.root.0.pages.replace(vec!["a-rebuilt".into(),"b-rebuilt".into()]);app.workspace.root.set_current_page(Some(0));app.workspace.applying.set(false);
'''
if 'fn remember_current_navigation_page(' in t13:
    main += 'app.remember_current_navigation_page();\n'
main += r'''
app.handlers.invoke("mru_toggle",&json!({})).unwrap();let rebuilt=app.workspace.root.current_page();
let before=app.mru_pages.borrow().clone();app.closed.store(true,Ordering::Release);app.workspace.root.set_current_page(Some(0));assert_eq!(*app.mru_pages.borrow(),before);assert!(app.handlers.invoke("next_tab",&json!({})).is_err());
app.shutdown_protocol();assert!(app.protocol_cancellable.0.get());assert!(app.interaction_controllers.borrow().is_empty());assert!(app.mru_pages.borrow().is_empty());assert!(app.handlers.invoke("mru_toggle",&json!({})).is_err());assert!(app.bridge_queue.borrow_mut().push(comandos_app::ui::bridge::parse_bridge(r#"{"session":"safe"}"#).unwrap()).is_err());assert!(app.notebook.0.signals.borrow().is_empty());assert!(app.workspace.root.0.signals.borrow().is_empty());
let app=App::new(true);app.install_app_commands();let weak=Rc::downgrade(&app);let root=app.workspace.root.clone();let registry_count=app.protocol_signals.borrow().len();drop(app);assert!(weak.upgrade().is_none());root.set_current_page(Some(1));
let mut xterm=vec![];for session in ["local".to_string(),"safe_-09".into(),"x".repeat(32),"proj.name".into(),"x".repeat(33),"x".repeat(81),"bad session".into(),"".into()]{let app=App::new(true);app.install_app_commands();let result=app.handlers.invoke("open_xterm_tab",&json!({"session":session}));xterm.push(json!({"session":session,"success":result.is_ok(),"effects":app.events.borrow().clone()}));}
println!("{}",json!({"modes":results,"reparent":reparent,"rebuilt":rebuilt,"owned_signals":registry_count,"xterm":xterm}));}
'''
workspace_path = checkout / 'crates/comandos-app/src/ui/workspace.rs'
workspace_source = workspace_path.read_text()
workspace_methods = '\n'.join(block(workspace_source, f'    pub fn {name}(') for name in ['page_key', 'page_index', 'focus_page', 'select'])
workspace_template = Path(__file__).with_name('t13_workspace_probe.rs').read_text()
workspace_fixture = workspace_template.replace('// EXACT_WORKSPACE_METHODS', workspace_methods)
main = main.replace('fn main(){', 'fn main(){workspace_probe::run();')
source = '#![forbid(unsafe_code)]\n' + workspace_fixture + preamble + methods + main
(out / 'src/main.rs').write_text(source)
manifest = f'''[package]\nname = "gtk-t13-repair-adapter-probe"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[patch.crates-io]\nalacritty_terminal = {{ path = {json.dumps(str(checkout / 'vendor/alacritty_terminal'))} }}\n[dependencies]\ncomandos-app = {{ path = {json.dumps(str(checkout / 'crates/comandos-app'))} }}\ncomandos-core = {{ path = {json.dumps(str(checkout / 'crates/comandos-core'))} }}\nserde_json = "=1.0.150"\n'''
(out / 'Cargo.toml').write_text(manifest)
build = subprocess.run(['nice','-n10','cargo','build','--offline','--manifest-path',str(out/'Cargo.toml'),'-j2'],timeout=180,capture_output=True,text=True)
(out/'build.log').write_text(build.stdout+build.stderr)
assert build.returncode==0,build.stdout+build.stderr
home=out/'home';home.mkdir(exist_ok=True)
xdg=out/'xdg';xdg.mkdir(exist_ok=True)
env={'HOME':str(home),'XDG_CONFIG_HOME':str(xdg),'XDG_DATA_HOME':str(xdg),'XDG_RUNTIME_DIR':str(xdg),'PATH':'/usr/bin:/bin','LC_ALL':'C.UTF-8','COMANDOS_APP_TMUX_SOCKET':str(home/'private.tmux'),'LD_LIBRARY_PATH':os.environ.get('LD_LIBRARY_PATH','')}
binary=Path(os.environ['CARGO_TARGET_DIR'])/'debug/gtk-t13-repair-adapter-probe'
run=subprocess.run([str(binary)],env=env,cwd=out,timeout=25,capture_output=True,text=True)
(out/'run.log').write_text(run.stdout+run.stderr)
assert run.returncode==0,run.stdout+run.stderr
result=json.loads(run.stdout)
result['source_hashes']={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [app_path,t13_path,workspace_path,Path(__file__).resolve(),Path(__file__).with_name('t13_workspace_probe.rs'),out/'src/main.rs']}
(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
for mode in result['modes']:
    assert mode['tracking'],mode
    for nav in mode['navigation']:assert nav['success'] and nav['current']==nav['want'],nav
assert result['reparent']==[[1,1],[0,0],[1,1],[0,0]],result['reparent']
assert result['rebuilt']==1,result['rebuilt']
assert result['owned_signals']==2,result['owned_signals']
for case in result['xterm']:
    session=case['session'];valid=1<=len(session)<=32 and all(c.isascii() and (c.isalnum() or c in '_-') for c in session)
    key='xterm-'+(session if valid else 'local')
    assert case['success'] and case['effects'][0][1]==key and case['effects'][1]==['select',key] and case['effects'][2]==['persist'],case
# Guard the actual production hooks and teardown, not only their doubles.
assert 'self.remember_current_navigation_page();' in block(app,'    fn apply_workspace(')
assert 'self.tabs_added();' in block(app,'    fn add_tab(')
assert 'self.remember_current_navigation_page();' in block(app,'    fn tabs_added(')
assert 'object.disconnect(signal);' in block(t13,'    pub(super) fn shutdown_protocol(')
assert 'self.mru_pages.borrow_mut().clear();' in block(t13,'    pub(super) fn shutdown_protocol(')
assert 'app.active_notebook()' in app
assert 'self.forget_navigation_page(key);' in block(app, '    fn close_tab(')
print('PASS exact owned adapter: legacy/workspace cycle, MRU signals, hide/show, reparent/rebuild, closed guards, weak ownership, disconnect, nested workspace focus/ancestry, xterm effects')
