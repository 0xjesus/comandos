#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use comandos_app::ui::header::{HEADER_ACTIONS, tween};
#[path = "support/header_models_oracle.rs"]
mod header_original;
#[test]
fn all_original_header_actions_keep_their_owners() {
    assert_eq!(HEADER_ACTIONS, ["quickTerminal", "newSession", "sortMenu"]);
    assert_eq!(tween(0., 0., 1., 150.), 0.);
    assert_eq!(tween(150000., 0., 1., 150.), 1.);
}
#[path = "support/t14_adapter.rs"]
mod adapter;
#[test]
fn exact_owned_callbacks_keep_guards_generations_and_window_ownership() {
    let native = adapter::execute();
    compare_original_header(&native);
}

fn compare_original_header(native: &serde_json::Value) {
    use comandos_app::theme::{button_style_css, desktop_theme, header_css, themes_from_file};
    use serde_json::json;
    let script = r#"import ast,json,sys,os,types
import gi
gi.require_version('GdkPixbuf','2.0')
from gi.repository import GdkPixbuf
source=open(sys.argv[1]).read();tree=ast.parse(source)
ns={'os':os,'GdkPixbuf':GdkPixbuf,'_ICONS_DIR':os.path.join(os.path.dirname(os.path.dirname(sys.argv[1])),'dash','icons')}
ns['Gtk']=types.SimpleNamespace(Image=types.SimpleNamespace(new_from_pixbuf=lambda pb:pb,new_from_icon_name=lambda *args:None),IconSize=types.SimpleNamespace(SMALL_TOOLBAR=1))
names={'PALETTE','PAL_DIA','PAL_CALIDO','PAL_BRUNO','PAL_UBUNTU','THEMES','_build_hb_css','button_style_css','_themed_icon_image','_tween','_open_wizard'}
actions=[]
for n in tree.body:
 if isinstance(n,ast.Assign):
  for target in n.targets:
   if isinstance(target,ast.Name) and target.id=='HEADER_ACTIONS':actions.extend(ast.literal_eval(k) for k in n.value.keys)
   if isinstance(target,ast.Subscript) and isinstance(target.value,ast.Name) and target.value.id=='HEADER_ACTIONS':actions.append(ast.literal_eval(target.slice))
 if (isinstance(n,ast.FunctionDef) and n.name in names) or (isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id in names for t in n.targets)):exec(compile(ast.Module([n],[]),sys.argv[1],'exec'),ns)
out={'actions':actions,'rasters':[],'themes':[]}
for name in ['noche','dia','calido','termius','bruno','superglass','neon','contraste','ubuntu']:
 ns['THEME']=ns['THEMES'][name]
 out['themes'].append({'header':ns['_build_hb_css']().decode(),'buttons':[ns['button_style_css'](style,ns['THEME']).decode() for style in ['sutil','arcade','tecla','pixel','consola']]})
 for icon in ['plus','terminal','arrow-up-down','rows','chevron-left','chevron-right','panel-left','close','minimize','maximize','timer','bell','settings','sparkles']:
  for size in [12,16,18,20]:
   pb=ns['_themed_icon_image'](icon,size)
   out['rasters'].append([name,icon,size,pb.get_width(),pb.get_height(),list(pb.get_pixels())])
class Widget:
 def add_tick_callback(self,fn):self.tick=fn
widget=Widget();values=[];done=[];ns['_tween'](widget,values.append,2,8,150,lambda:done.append(True))
for frame in [1000000,1020000,1075000,1150000]:widget.tick(widget,types.SimpleNamespace(get_frame_time=lambda:frame))
out['tween']=values;out['done']=done
visible=[];scripts=[];ns['nb']=types.SimpleNamespace(set_visible=visible.append);ns['wv']=types.SimpleNamespace(run_javascript=lambda script,*args:scripts.append(script));ns['_open_wizard']();out['wizard']=[visible,scripts]
print(json.dumps(out))
"#;
    let expected = header_original::oracle(script);
    let rasters = native["rasters"].as_array().unwrap();
    assert_eq!(rasters.len(), 504);
    for (i, raster) in rasters.iter().enumerate() {
        assert_eq!(raster, &expected["rasters"][i], "themed icon case {i}");
    }
    let themes = themes_from_file(None);
    for (i, name) in [
        "noche",
        "dia",
        "calido",
        "termius",
        "bruno",
        "superglass",
        "neon",
        "contraste",
        "ubuntu",
    ]
    .into_iter()
    .enumerate()
    {
        let theme = desktop_theme(name, &themes).unwrap();
        let buttons: Vec<_> = ["sutil", "arcade", "tecla", "pixel", "consola"]
            .map(|style| button_style_css(style, &theme))
            .into();
        assert_eq!(
            json!({"header":header_css(&theme),"buttons":buttons}),
            expected["themes"][i],
            "header and styles {name}"
        );
    }
    for (i, elapsed) in [0., 20000., 75000., 150000.].into_iter().enumerate() {
        assert!(
            (tween(elapsed, 2., 8., 150.) - expected["tween"][i].as_f64().unwrap()).abs() < 1e-12
        );
    }
    assert_eq!(expected["done"], json!([true]));
    assert_eq!(
        expected["wizard"],
        json!([[true], ["try{nsOpen()}catch(e){}"]])
    );
}
#[test]
fn tween_clock_starts_at_first_owned_frame_and_finishes_once() {
    let mut clock = comandos_app::ui::header::Tween::new(2., 8., 150.);
    assert_eq!(clock.step(1_000_000.), (2., false));
    assert_eq!(clock.step(1_075_000.), (7.25, false));
    assert_eq!(clock.step(1_150_000.), (8., true));
}
