//! Corte tabs, T3: crear y revivir sesiones (`/recover-tab`, `/ensure`,
//! `/new`, `/shell`, `/up`) contra el Python (gemelo).
//!
//! Confinamiento: cada lado del gemelo tiene su HOME temporal corto, su tmux
//! privado (`-S`) y su `fakebin` confinado (`PATH` = solo él). Las sesiones
//! que crean las rutas nacen por el `systemd-run` falso del `fakebin` (anota y
//! solo ejecuta colas de tmux, sin unidades) y el guardián de tmux (`-S` al
//! socket privado, `env -i` con el entorno confinado): su panel corre `bash`
//! con el `claude`/`codex`/`ssh` FALSOS del `fakebin`, que solo anotan. La
//! terminal de `spawn_terminal` y `wmctrl` también son falsos que anotan. Cada
//! prueba que crea sesiones empieza por `assert_confined` (canario). Los
//! clientes de control (`tmux -C attach`) los lanza la prueba con
//! `TestHome::tmux_command` (`-S`) y los mata ella misma (su propio proceso
//! cliente; nunca un `kill-server`/`kill-session`). La limpieza es el `Drop`
//! de cada `TestHome` (`kill-server -S` y después borrar).
mod support;

use std::{
    process::{Child, Stdio},
    time::Duration,
};
use support::{
    FakeLegacy, TestHome, fake_scope_calls, front,
    oracle::fake_calls,
    request_body, run_tmux,
    tabs::{normalize_home, options_for, read_normalized, seed_registry},
    twin::{Twin, normalize},
};

/// Los archivos que escriben estas rutas.
const FILES: [&str; 5] = [
    "app-tabs.json",
    "app-tabs-meta.json",
    "app-tabs-history.json",
    "app-tab-open.json",
    "app-focus.json",
];

/// Canario antes de crear nada: todo lo que una sesión nueva puede lanzar en
/// los dos lados es un falso del `fakebin`, el tmux lleva `-S` al socket
/// privado y el `systemd-run` es el falso que solo ejecuta tmux.
fn assert_confined(t: &Twin) {
    for home in [&t.a, &t.b] {
        let fakebin = home.root.join("fakebin");
        for fake in [
            "claude",
            "codex",
            "grok",
            "opencode",
            "gemini",
            "agy",
            "aider",
            "cc-acp",
            "ssh",
            "kitty",
            "tilix",
            "gnome-terminal",
            "wmctrl",
        ] {
            let path = fakebin.join(fake);
            assert!(
                !path.is_symlink(),
                "{fake} es un enlace: {}",
                path.display()
            );
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains("fakebin.log"),
                "{fake} no es el falso: {text}"
            );
        }
        let socket = comandos_server::dash::native::tmux::private_socket(&home.tmux_dir());
        let guard = std::fs::read_to_string(fakebin.join("tmux")).unwrap();
        assert!(
            guard.contains(&format!(
                "-S {}",
                support::oracle::sh_quote(&socket.display().to_string())
            )),
            "guardián sin -S privado: {guard}"
        );
        assert!(guard.contains(" -i "), "guardián sin env -i: {guard}");
        let scope = std::fs::read_to_string(fakebin.join("systemd-run")).unwrap();
        assert!(
            scope.contains("faltan las banderas de scope_cmd"),
            "systemd-run no es el falso: {scope}"
        );
        let path = home
            .confined_env()
            .into_iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v);
        assert_eq!(path, Some(fakebin.display().to_string()));
    }
    let opts = &t.front_options;
    let fakebin = t.a.root.join("fakebin");
    assert_eq!(opts.tmux.program.path, fakebin.join("tmux"));
    support::assert_private_tmux(opts);
    let scope = opts.scope.as_ref().expect("scope del gemelo");
    assert_eq!(scope.path, fakebin.join("systemd-run"));
    assert!(scope.env_clear, "el scope del frente hereda el entorno");
    assert!(
        t.b.root.join(".comandos-twin-guard").exists(),
        "el prólogo confinado del oráculo no corrió"
    );
}

/// Texto comparable entre los dos HOME: reloj normalizado y raíz como `~`.
fn comparable(home: &TestHome, text: &str) -> String {
    let text = normalize_home(home, &normalize(text));
    regex::Regex::new(r"profile-mcp-[A-Za-z0-9_-]+\.json")
        .unwrap()
        .replace_all(&text, "profile-mcp-ID.json")
        .into_owned()
}

/// Un argumento de una llamada a un falso o a tmux, comparable: la ruta de un
/// ejecutable del `fakebin` por su nombre, los clientes de control por `N`.
fn arg_of(home: &TestHome, arg: &str) -> String {
    let fakebin = format!("{}/", home.root.join("fakebin").display());
    let arg = arg.strip_prefix(&fakebin).unwrap_or(arg);
    let arg = regex::Regex::new(r"client-[0-9]+")
        .unwrap()
        .replace_all(arg, "client-N");
    comparable(home, &arg)
}

/// Las órdenes de tmux que mutan, comparables, y vacía el registro.
fn take_mutations(t: &Twin) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let side = |home: &TestHome, calls: Vec<Vec<String>>| -> Vec<Vec<String>> {
        let _ = std::fs::remove_file(home.root.join("tmux.log"));
        calls
            .into_iter()
            .map(|c| c.iter().map(|a| arg_of(home, a)).collect())
            .collect()
    };
    let (a, b) = (t.tmux_mutations_a(), t.tmux_mutations_b());
    (side(&t.a, a), side(&t.b, b))
}

/// Llamadas al `systemd-run` falso, comparables: sin el prefijo del tmux del
/// frente (`-f /dev/null -S <socket>`, el Python no lo pasa).
fn scope_calls(home: &TestHome) -> Vec<Vec<String>> {
    fake_scope_calls(home)
        .into_iter()
        .map(|call| {
            let mut args: Vec<String> = call.iter().map(|a| arg_of(home, a)).collect();
            if let Some(at) = args.iter().position(|a| a == "tmux") {
                while args
                    .get(at + 1)
                    .is_some_and(|flag| flag == "-f" || flag == "-S")
                {
                    args.drain(at + 1..(at + 3).min(args.len()));
                }
            }
            args
        })
        .collect()
}

/// Llamadas a los falsos del `fakebin` con nombre en `names`, comparables.
fn fakes(home: &TestHome, names: &[&str]) -> Vec<Vec<String>> {
    fake_calls(&home.root)
        .into_iter()
        .filter(|c| names.contains(&c.name.as_str()))
        .map(|c| {
            let mut row = vec![c.name.clone()];
            row.extend(c.args.iter().map(|a| arg_of(home, a)));
            row
        })
        .collect()
}

/// La misma petición a los dos lados: estado, cuerpo (raíz como `~`),
/// archivos y órdenes de tmux que mutan iguales. Devuelve el estado.
async fn same(t: &Twin, path: &str, body: &str) -> u16 {
    same_logged(t, path, body).await.0
}

/// `same`, devolviendo además las órdenes de tmux que mutan (iguales).
async fn same_logged(t: &Twin, path: &str, body: &str) -> (u16, Vec<Vec<String>>) {
    take_mutations(t);
    let run = t.post(path, body).await;
    assert_eq!(
        run.front.status,
        run.oracle.status,
        "{path} {body}: {} ≠ {}",
        run.front.text(),
        run.oracle.text()
    );
    assert_eq!(
        comparable(&t.a, &run.front.text()),
        comparable(&t.b, &run.oracle.text()),
        "cuerpo de {path} {body}"
    );
    for name in FILES {
        assert_eq!(
            read_normalized(&t.a, name),
            read_normalized(&t.b, name),
            "{name} tras {path} {body}"
        );
    }
    let (a, b) = take_mutations(t);
    assert_eq!(a, b, "órdenes de tmux de {path} {body}");
    (run.front.status, a)
}

fn argv(calls: &[&[&str]]) -> Vec<Vec<String>> {
    calls
        .iter()
        .map(|c| c.iter().map(|a| (*a).to_owned()).collect())
        .collect()
}

/// `#{session_name}:#{window_name}` de todas las ventanas (vacío sin servidor).
/// Los nombres que no fijan las rutas (`claude`, `shell`) los pone el
/// `automatic-rename` de tmux según el proceso del momento: cuentan como `*`.
fn windows(home: &TestHome) -> Vec<String> {
    let out = home
        .tmux_command()
        .args(["list-windows", "-a", "-F", "#{session_name}:#{window_name}"])
        .output()
        .unwrap();
    let mut lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| match line.rsplit_once(':') {
            Some((sess, name)) if name != "claude" && name != "shell" => format!("{sess}:*"),
            _ => line.to_owned(),
        })
        .collect();
    lines.sort();
    lines
}

/// Espera a que los dos `fakebin.log` tengan la llamada `name` (el panel corrió
/// el falso); devuelve cuántas hay en A.
async fn wait_fake(t: &Twin, name: &str, count: usize) {
    for _ in 0..200 {
        let a = fakes(&t.a, &[name]).len();
        let b = fakes(&t.b, &[name]).len();
        if a >= count && b >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("el falso {name} no corrió {count} veces en los dos lados");
}

#[tokio::test]
async fn create_and_revive_match_python() {
    let Some(t) = Twin::start("sess", |h| {
        seed_registry(h);
        run_tmux(h, &["new-session", "-d", "-s", "ssh-x", "cat"]);
    })
    .await
    else {
        return;
    };
    assert_confined(&t);
    // Proyecto: ventana del agente (el `claude` falso) y `shell`.
    let launch = "set -a; . \"$HOME/.claude/hooks/providers.env\" 2>/dev/null; set +a; \
                  claude --continue 2>/dev/null || claude; exec $SHELL";
    let (status, log) = same_logged(&t, "/new", r#"{"session":"p2f"}"#).await;
    assert_eq!(status, 200);
    assert_eq!(
        log,
        argv(&[
            &[
                "new-session",
                "-d",
                "-s",
                "p2f",
                "-n",
                "claude",
                "-c",
                "~/codebase/p2f",
                launch
            ],
            &[
                "new-window",
                "-d",
                "-t",
                "=p2f",
                "-n",
                "shell",
                "-c",
                "~/codebase/p2f"
            ],
        ])
    );
    let scoped = scope_calls(&t.a);
    assert_eq!(
        scoped
            .first()
            .map(|c| c.iter().take(6).cloned().collect::<Vec<_>>()),
        Some(
            [
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "tmux",
                "new-session"
            ]
            .map(String::from)
            .to_vec()
        ),
        "{scoped:?}"
    );
    wait_fake(&t, "claude", 1).await;
    // Ya viva: solo el registro.
    assert_eq!(same(&t, "/new", r#"{"session":"p2f"}"#).await, 200);
    // Sin proyecto: shell en el HOME.
    assert_eq!(same(&t, "/new", r#"{"session":"suelta"}"#).await, 200);
    assert_eq!(
        same(&t, "/ensure", r#"{"session":"p2f","win":"shell"}"#).await,
        200
    );
    // ssh: identidad derivada y la ventana `shell` con otra conexión (el
    // `ssh` falso, que falla).
    assert_eq!(
        same(&t, "/ensure", r#"{"session":"ssh-x","win":"shell"}"#).await,
        200
    );
    wait_fake(&t, "ssh", 1).await;
    // Sin clientes: la terminal falsa por el `systemd-run` falso.
    assert_eq!(same(&t, "/shell", r#"{"session":"p2f"}"#).await, 200);
    assert_eq!(same(&t, "/up", r#"{"session":"p2f"}"#).await, 200);
    // Revivir con otro agente (el `codex` falso) y una que no tiene carpeta.
    assert_eq!(
        same(
            &t,
            "/recover-tab",
            r#"{"session":"rec","label":"p2f","agent":"codex"}"#
        )
        .await,
        200
    );
    wait_fake(&t, "codex", 1).await;
    assert_eq!(
        same(&t, "/recover-tab", r#"{"session":"rec2","label":"nada"}"#).await,
        400
    );
    // Agente fuera de `agent_set()`: `claude`.
    assert_eq!(
        same(
            &t,
            "/recover-tab",
            r#"{"session":"rec3","label":"p2f","agent":"nave"}"#
        )
        .await,
        200
    );
    wait_fake(&t, "claude", 2).await;
    // `/ensure` revive con el `cwd` pedido.
    let cwd = |h: &TestHome| h.root.join("codebase/p2f").display().to_string();
    let body_a = format!(r#"{{"session":"otra-e","cwd":"{}"}}"#, cwd(&t.a));
    let body_b = format!(r#"{{"session":"otra-e","cwd":"{}"}}"#, cwd(&t.b));
    let (fa, fb) = (
        t.post_front("/ensure", &body_a).await,
        t.post_oracle("/ensure", &body_b).await,
    );
    assert_eq!((fa.status, fa.text()), (fb.status, fb.text()));
    for name in FILES {
        assert_eq!(read_normalized(&t.a, name), read_normalized(&t.b, name));
    }
    let (ma, mb) = take_mutations(&t);
    assert_eq!(ma, mb, "órdenes de tmux de /ensure con cwd");
    wait_fake(&t, "claude", 3).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(windows(&t.a), windows(&t.b), "ventanas");
    assert_eq!(scope_calls(&t.a), scope_calls(&t.b), "systemd-run");
    let agents = ["claude", "codex", "ssh", "kitty", "wmctrl"];
    let (mut fa, mut fb) = (fakes(&t.a, &agents), fakes(&t.b, &agents));
    fa.sort();
    fb.sort();
    assert_eq!(fa, fb, "falsos lanzados");
    // Las terminales se lanzaron por el `systemd-run` falso con sus banderas.
    assert!(
        scope_calls(&t.a).iter().any(|c| c.starts_with(&[
            "--user".into(),
            "--collect".into(),
            "--quiet".into(),
            "kitty".into()
        ])),
        "{:?}",
        scope_calls(&t.a)
    );
}

#[tokio::test]
async fn up_without_dir_writes_nothing() {
    let Some(t) = Twin::start("up-nodir", seed_registry).await else {
        return;
    };
    assert_confined(&t);
    let before: Vec<Option<String>> = FILES.iter().map(|f| read_normalized(&t.a, f)).collect();
    assert_eq!(same(&t, "/up", r#"{"session":"inexistente"}"#).await, 400);
    for (status, path) in [(400, "/shell"), (400, "/ensure")] {
        assert_eq!(same(&t, path, r#"{"session":"inexistente"}"#).await, status);
    }
    assert!(windows(&t.a).is_empty() && windows(&t.b).is_empty());
    let after: Vec<Option<String>> = FILES.iter().map(|f| read_normalized(&t.a, f)).collect();
    assert_eq!(before, after, "el registro no cambia");
    assert!(fake_scope_calls(&t.a).is_empty() && fake_scope_calls(&t.b).is_empty());
}

#[tokio::test]
async fn no_scope_in_production_declines_before_creating() {
    if !support::tmux_available() {
        return;
    }
    let home = TestHome::new_short("sess-noscope");
    seed_registry(&home);
    let legacy = FakeLegacy::start().await;
    let mut opts = options_for(&home);
    opts.scope = None;
    let fr = front(&home, legacy.port, opts).await;
    for path in ["/new", "/ensure", "/shell", "/up", "/recover-tab"] {
        let wire = request_body(fr.port, "POST", path, "", r#"{"session":"p2f"}"#).await;
        assert_eq!(wire.text(), r#"{"legacy": true}"#, "{path}");
    }
    assert_eq!(legacy.requests().len(), 5);
    // Ni una orden de tmux, ni un servidor, ni el registro tocado.
    assert!(!home.root.join("tmux.log").exists());
    let socket = comandos_server::dash::native::tmux::private_socket(&home.tmux_dir());
    assert!(!socket.exists(), "nació un servidor tmux");
    assert!(!support::tabs::exists(&home, "app-tab-open.json"));
    fr.stop().await;
}

/// Un cliente de control (`tmux -C attach`, sin terminal) en `sess` del
/// servidor privado de `home`. La prueba lo mata con `stop_client`.
fn control_client(home: &TestHome, sess: &str) -> Child {
    let target = format!("={sess}");
    home.tmux_command()
        .args(["-C", "attach", "-t", &target])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Mata el proceso cliente que lanzó la prueba (no el servidor ni sesiones).
fn stop_client(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Espera a que `list-clients` del servidor de `home` cumpla `want`.
async fn wait_clients(home: &TestHome, want: impl Fn(&str) -> bool) {
    for _ in 0..200 {
        let out = home
            .tmux_command()
            .args(["list-clients", "-F", "#{session_name}"])
            .output()
            .unwrap();
        if want(&String::from_utf8_lossy(&out.stdout)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("los clientes de control no llegaron al estado esperado");
}

#[tokio::test]
async fn focus_with_clients_matches_python() {
    let Some(t) = Twin::start("sess-focus", |h| {
        seed_registry(h);
        for sess in ["p2f", "local", "otra"] {
            run_tmux(h, &["new-session", "-d", "-s", sess, "cat"]);
        }
    })
    .await
    else {
        return;
    };
    assert_confined(&t);
    // App abierta (cliente en `local`): `app-focus.json` y la ventana de la app.
    let (ca, cb) = (control_client(&t.a, "local"), control_client(&t.b, "local"));
    for home in [&t.a, &t.b] {
        wait_clients(home, |s| s.contains("local")).await;
    }
    assert_eq!(same(&t, "/up", r#"{"session":"p2f"}"#).await, 200);
    assert!(support::tabs::exists(&t.a, "app-focus.json"));
    stop_client(ca);
    stop_client(cb);
    for home in [&t.a, &t.b] {
        wait_clients(home, |s| s.trim().is_empty()).await;
    }
    // Sin app y sin clientes en `p2f`: el cliente más activo cambia a `p2f`.
    let (ca, cb) = (control_client(&t.a, "otra"), control_client(&t.b, "otra"));
    for home in [&t.a, &t.b] {
        wait_clients(home, |s| s.contains("otra")).await;
    }
    assert_eq!(same(&t, "/shell", r#"{"session":"p2f"}"#).await, 200);
    // El pulso keep-above termina 0,6 s después.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    stop_client(ca);
    stop_client(cb);
    let (wa, wb) = (fakes(&t.a, &["wmctrl"]), fakes(&t.b, &["wmctrl"]));
    assert_eq!(wa, wb, "wmctrl");
    assert!(
        wa.iter().any(|c| c.iter().any(|a| a == "remove,above")),
        "{wa:?}"
    );
    assert_eq!(windows(&t.a), windows(&t.b), "ventanas");
}

#[tokio::test]
async fn bad_inputs_match_python() {
    let Some(t) = Twin::start("sess-bad", seed_registry).await else {
        return;
    };
    assert_confined(&t);
    for (path, body) in [
        ("/new", r#"{"session":"mal nombre"}"#),
        ("/recover-tab", r#"{"session":"a b"}"#),
        ("/recover-tab", r#"{"session":5}"#),
        ("/ensure", r#"{"session":["x"]}"#),
        // Agente que no es texto al crear: el `agent.replace` del Python (500).
        ("/new", r#"{"session":"p2f","agent":5}"#),
    ] {
        same(&t, path, body).await;
    }
    assert!(windows(&t.a).is_empty() && windows(&t.b).is_empty());
}

/// `ssh` falso de las pruebas SSH (los dos `fakebin` y el `opts.ssh` del
/// frente): anota en `$HOME/fakebin.log` (el HOME confinado de cada lado) y
/// nunca abre una conexión. `-O check` sale con 0 solo para `tunel` (túnel de
/// control vivo); la prueba de llave (`… <host> true`) sale con 0 para `bueno`,
/// con 255 y `Permission denied` para `malo` y con 255 y un plazo vencido para
/// `tunel`; cualquier otra llamada (la del panel, `ssh <host>`) sale con 255.
const FAKE_SSH: &str = "#!/bin/sh\n\
printf '%s\\0' ssh \"$@\" \"$(printf '\\036')\" >> \"$HOME/fakebin.log\"\n\
if [ \"$1\" = -O ]; then [ \"$3\" = tunel ] && exit 0; exit 255; fi\n\
case \"$*\" in\n\
  *' bueno true') exit 0 ;;\n\
  *' malo true') echo 'malo: Permission denied (publickey).' >&2; exit 255 ;;\n\
  *' tunel true') echo 'ssh: connect to host tunel port 22: Connection timed out' >&2; exit 255 ;;\n\
esac\n\
exit 255\n";

/// `~/.ssh/config` del HOME temporal con los tres hosts de las pruebas y el
/// `ssh` falso de nuevo en el `fakebin` (`seed_registry` reinstala el que
/// siempre falla).
fn seed_ssh(home: &TestHome) {
    use std::os::unix::fs::PermissionsExt;
    seed_registry(home);
    let fake = home.root.join("fakebin/ssh");
    std::fs::write(&fake, FAKE_SSH).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        home.root.join(".ssh/config"),
        "Host bueno\n  HostName 10.0.0.2\n\nHost malo\n  User u\n\nHost tunel\n",
    )
    .unwrap();
}

/// Nombres de las sesiones del servidor privado de `home`, ordenados.
fn sessions_of(home: &TestHome) -> Vec<String> {
    let out = home
        .tmux_command()
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap();
    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn ssh_routes_match_python() {
    let Some(t) = Twin::start_with(
        "ssh",
        seed_ssh,
        support::twin::TwinOpts {
            fakebin_extra: vec![("ssh".into(), FAKE_SSH.into())],
            ..Default::default()
        },
    )
    .await
    else {
        return;
    };
    assert_confined(&t);
    // Crear: prueba de llave buena, sesión `ssh-bueno` con el `ssh` falso.
    let (status, log) = same_logged(&t, "/ssh-connect", r#"{"host":"bueno"}"#).await;
    assert_eq!(status, 200);
    assert_eq!(
        log,
        argv(&[
            &[
                "new-session",
                "-d",
                "-s",
                "ssh-bueno",
                "-n",
                "ssh",
                "ssh bueno; exec $SHELL"
            ],
            &["set-option", "-t", "ssh-bueno", "mouse", "off"],
        ])
    );
    wait_fake(&t, "ssh", 2).await;
    // Repetida: la sesión existe y su ssh murió (el falso sale): se reintenta.
    let (status, log) = same_logged(&t, "/ssh-connect", r#"{"host":"bueno"}"#).await;
    assert_eq!(status, 200);
    assert_eq!(
        log,
        argv(&[
            &["set-option", "-t", "ssh-bueno", "mouse", "off"],
            &["send-keys", "-t", "=ssh-bueno:", "-l", "--", "ssh bueno"],
            &["send-keys", "-t", "=ssh-bueno:", "Enter"],
        ])
    );
    // Llave rechazada y host que no responde; repetida con túnel vivo: «mux».
    for body in [
        r#"{"host":"malo"}"#,
        r#"{"host":"tunel"}"#,
        r#"{"host":"tunel"}"#,
    ] {
        assert_eq!(same(&t, "/ssh-connect", body).await, 200, "{body}");
    }
    // Desconocidos y entradas raras: sin efectos (o el 500 del Python).
    for body in [
        r#"{"host":"desconocido"}"#,
        r#"{"host":"mal host"}"#,
        r#"{}"#,
        r#"{"host":null}"#,
        r#"{"host":5}"#,
    ] {
        same(&t, "/ssh-connect", body).await;
        same(&t, "/ssh-new-tab", body).await;
    }
    // Pestañas nuevas: siempre una sesión más, `sshtab-<host>-<i>`.
    for body in [
        r#"{"host":"bueno"}"#,
        r#"{"host":"bueno"}"#,
        r#"{"host":"malo"}"#,
        r#"{"host":"tunel"}"#,
    ] {
        assert_eq!(same(&t, "/ssh-new-tab", body).await, 200, "{body}");
    }
    assert_eq!(sessions_of(&t.a), sessions_of(&t.b), "list-sessions");
    assert_eq!(
        sessions_of(&t.a),
        [
            "ssh-bueno",
            "ssh-malo",
            "ssh-tunel",
            "sshtab-bueno-1",
            "sshtab-bueno-2",
            "sshtab-malo-1",
            "sshtab-tunel-1"
        ]
    );
    // Llamadas al `ssh` falso: pruebas de llave, `-O check` y los paneles.
    wait_fake(&t, "ssh", 18).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mut fa, mut fb) = (fakes(&t.a, &["ssh"]), fakes(&t.b, &["ssh"]));
    fa.sort();
    fb.sort();
    assert_eq!(fa, fb, "llamadas a ssh");
    assert_eq!(scope_calls(&t.a), scope_calls(&t.b), "systemd-run");
    assert_eq!(windows(&t.a), windows(&t.b), "ventanas");
}

/// `T-YYYY-MM-DD-HH-MM-SS[-n]` (la carpeta fechada de la terminal rápida) como
/// `T-FECHA`: el frente de las pruebas tiene el reloj fijo y el Python, el real.
fn undated(text: &str) -> String {
    regex::Regex::new(r"T-[0-9]{4}(-[0-9]{2}){5}(-[0-9]+)?")
        .unwrap()
        .replace_all(text, "T-FECHA")
        .into_owned()
}

/// El documento de `GET /workspace` comparable: sin `revision`, con las
/// marcas de tiempo normalizadas y la carpeta fechada como `T-FECHA`.
fn workspace_doc(home: &TestHome, wire: &support::Wire) -> String {
    let mut doc: serde_json::Value = serde_json::from_slice(&wire.body).unwrap();
    if let Some(map) = doc.as_object_mut() {
        map.remove("revision");
    }
    undated(&comparable(home, &doc.to_string()))
}

#[tokio::test]
async fn quick_terminal_outside_sidebar_registers_tab() {
    let Some(t) = Twin::start_with(
        "quick-tab",
        seed_registry,
        support::twin::TwinOpts {
            // La misma base que el Python (`default_base()` en su HOME).
            front: Some(Box::new(|o| {
                o.quick_base = o.home.join("codebase/0xJesus/Terminal");
            })),
            ..Default::default()
        },
    )
    .await
    else {
        return;
    };
    assert_confined(&t);
    let body = r#"{"requestId":"rq-2f-0001"}"#;
    for created in [true, false] {
        take_mutations(&t);
        let run = t.post("/terminal/quick", body).await;
        assert_eq!(
            (run.front.status, run.oracle.status),
            (200, 200),
            "{} / {}",
            run.front.text(),
            run.oracle.text()
        );
        assert_eq!(
            undated(&comparable(&t.a, &run.front.text())),
            undated(&comparable(&t.b, &run.oracle.text())),
            "cuerpo"
        );
        assert!(
            run.front
                .text()
                .contains(&format!("\"created\": {created}"))
        );
        for name in FILES {
            assert_eq!(
                read_normalized(&t.a, name).map(|s| undated(&s)),
                read_normalized(&t.b, name).map(|s| undated(&s)),
                "{name}"
            );
        }
        let (a, b) = take_mutations(&t);
        assert_eq!(
            a.iter().map(|c| undated(&c.join(" "))).collect::<Vec<_>>(),
            b.iter().map(|c| undated(&c.join(" "))).collect::<Vec<_>>(),
            "órdenes de tmux"
        );
    }
    let meta = read_normalized(&t.a, "app-tabs-meta.json").unwrap();
    assert!(meta.contains(r#""kind": "scratch""#), "{meta}");
    let ws = t.get("/workspace").await;
    assert_eq!((ws.front.status, ws.oracle.status), (200, 200));
    let (wa, wb) = (
        workspace_doc(&t.a, &ws.front),
        workspace_doc(&t.b, &ws.oracle),
    );
    assert_eq!(wa, wb, "workspace");
    assert!(wa.contains("term-q"), "{wa}");
    assert_eq!(sessions_of(&t.a), sessions_of(&t.b), "list-sessions");
}

/// Un `app-tabs-meta.json` que el frente no lee con certeza (anidamiento ≥
/// 1000): la terminal rápida fuera de la barra y `/ssh-new-tab` responden el
/// 500 del ruling 2 ANTES de reclamar, probar la llave o crear la sesión; con
/// el corte `tabs` apagado, las dos declinan. La terminal de la barra no lee
/// el registro y sigue igual.
#[tokio::test]
async fn unsure_registry_answers_500_before_effects() {
    if !support::tmux_available() {
        return;
    }
    let home = TestHome::new_short("sess-unsure");
    seed_ssh(&home);
    let deep = format!("{}{}", "[".repeat(1100), "]".repeat(1100));
    home.write("app-tabs-meta.json", &deep);
    let legacy = FakeLegacy::start().await;
    let mut opts = options_for(&home);
    opts.scope = Some(comandos_server::dash::native::quick::scope_program(
        home.root.join("fakebin/systemd-run"),
    ));
    opts.ssh = opts.program(home.root.join("fakebin/ssh"));
    opts.quick_base = home.root.join("Terminal");
    let fr = front(&home, legacy.port, opts.clone()).await;
    for (path, body) in [
        ("/terminal/quick", r#"{"requestId":"rq-incierto"}"#),
        ("/ssh-new-tab", r#"{"host":"bueno"}"#),
    ] {
        let wire = request_body(fr.port, "POST", path, "", body).await;
        assert_eq!(
            (wire.status, wire.text()),
            (500, r#"{"error": "Error interno del tablero"}"#.to_owned()),
            "{path}"
        );
    }
    fr.stop().await;
    assert!(legacy.requests().is_empty(), "{:?}", legacy.requests());
    assert!(fake_calls(&home.root).is_empty(), "ningún ssh");
    assert!(fake_scope_calls(&home).is_empty(), "ninguna sesión");
    assert!(!home.root.join("Terminal").exists(), "ningún reclamo");
    assert_eq!(
        std::fs::read_to_string(home.hooks().join("app-tabs-meta.json")).unwrap(),
        deep
    );
    // Corte `tabs` apagado: las dos rutas declinan sin leer nada.
    opts.cuts_off
        .insert(comandos_server::dash::native::Cut::Tabs);
    let fr = front(&home, legacy.port, opts).await;
    for (path, body) in [
        ("/terminal/quick", r#"{"requestId":"rq-incierto"}"#),
        ("/ssh-new-tab", r#"{"host":"bueno"}"#),
    ] {
        let wire = request_body(fr.port, "POST", path, "", body).await;
        assert_eq!(wire.text(), r#"{"legacy": true}"#, "{path}");
    }
    fr.stop().await;
    assert_eq!(legacy.requests().len(), 2);
    assert!(fake_calls(&home.root).is_empty());
}

// ------------------------------------------------------------- T4

/// Las filas de una tabla de la base de uso de `home` sin las columnas que
/// dependen del reloj (`id`, `effective_at`, `created_at`), con la raíz del
/// HOME como `~`.
fn usage_rows(home: &TestHome, table: &str) -> Vec<Vec<String>> {
    let Ok(conn) = rusqlite::Connection::open(home.usage_db()) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(&format!("select * from {table} order by rowid")) else {
        return Vec::new();
    };
    let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
    let mut rows = stmt.query([]).unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        let mut cells = Vec::new();
        for (i, name) in names.iter().enumerate() {
            if matches!(name.as_str(), "id" | "effective_at" | "created_at") {
                continue;
            }
            let cell: rusqlite::types::Value = row.get(i).unwrap();
            cells.push(comparable(home, &format!("{name}={cell:?}")));
        }
        out.push(cells);
    }
    out
}

/// Archivo `~/<rel>` de `home` comparable (contenido con la raíz como `~` y
/// modo), o `None`.
fn home_file(home: &TestHome, rel: &str) -> Option<(String, u32)> {
    use std::os::unix::fs::PermissionsExt;
    let path = home.root.join(rel);
    let mode = std::fs::metadata(&path).ok()?.permissions().mode() & 0o7777;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Some((comparable(home, &text), mode))
}

/// Los archivos de cuentas y de confianza que escriben estas rutas.
const ACCOUNT_FILES: [&str; 9] = [
    ".claude.json",
    ".claude-accounts",
    ".claude-accounts/relotto/.claude.json",
    ".claude-accounts/nueva",
    ".claude-accounts/nueva/settings.json",
    ".claude/settings.json",
    ".codex-accounts/cuenta-2/config.toml",
    ".codex-accounts/dos/config.toml",
    ".grok-accounts/g1",
];

/// `same` para una ruta que teclea 1,5 s después: espera 2 s y devuelve los
/// `send-keys` (iguales en los dos lados, raíz como `~`).
async fn same_typed(t: &Twin, path: &str, body: &str) -> (u16, Vec<Vec<String>>) {
    let (status, created) = same_logged(t, path, body).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (a, b) = take_mutations(t);
    let a = unfold_launchers(&t.a, a);
    assert_eq!(a, b, "tecleo de {path} {body}");
    for rel in ACCOUNT_FILES {
        assert_eq!(
            home_file(&t.a, rel),
            home_file(&t.b, rel),
            "{rel} tras {path} {body}"
        );
    }
    let mut all = created;
    all.extend(a);
    (status, all)
}

/// El frente teclea `. ~/.local/share/comandos/launch/<nombre>` donde el
/// Python teclea el comando entero (desviación del 15-oct): se sustituye por
/// el contenido del lanzador, que debe ser 0600 y terminar en salto de línea.
fn unfold_launchers(home: &TestHome, log: Vec<Vec<String>>) -> Vec<Vec<String>> {
    use std::os::unix::fs::PermissionsExt;
    const PREFIX: &str = ". ~/.local/share/comandos/launch/";
    log.into_iter()
        .map(|mut args| {
            if args.first().is_some_and(|v| v == "send-keys")
                && let Some(name) = args.get(3).and_then(|c| c.strip_prefix(PREFIX))
            {
                assert!(
                    name.chars().all(|c| c.is_ascii_alphanumeric() || "-.".contains(c)),
                    "nombre de lanzador ilegible: {name}"
                );
                let path = home.root.join(".local/share/comandos/launch").join(name);
                let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{name}");
                let text = std::fs::read_to_string(&path).unwrap();
                let command = text.strip_suffix('\n').expect("lanzador sin salto final");
                args[3] = arg_of(home, command);
            }
            args
        })
        .collect()
}

/// El comando de cada `send-keys` de un registro comparable.
fn typed(log: &[Vec<String>]) -> Vec<String> {
    log.iter()
        .filter(|args| args.first().is_some_and(|v| v == "send-keys"))
        .filter_map(|args| args.get(3).cloned())
        .collect()
}

#[tokio::test]
async fn session_new_and_account_add_match_python() {
    let Some(t) = Twin::start("snew", |h| {
        seed_registry(h);
        support::tabs::seed_accounts(h);
    })
    .await
    else {
        return;
    };
    assert_confined(&t);
    let mut commands = Vec::new();
    let mut run = async |path: &str, body: &str, want: u16| {
        let (status, log) = same_typed(&t, path, body).await;
        assert_eq!(status, want, "{path} {body}");
        commands.extend(typed(&log));
    };
    // Sin efectos.
    run("/session-new", r#"{"cwd":"relativa"}"#, 400).await;
    run("/session-new", r#"{"cwd":5}"#, 400).await;
    run("/session-new", r#"{"cwd":"~/codebase/nada"}"#, 409).await;
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","routeId":""}"#,
        409,
    )
    .await;
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","routeId":"nadie:nada"}"#,
        409,
    )
    .await;
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","routeId":"claude:claude","harnessAccount":"nadie"}"#,
        409,
    )
    .await;
    // Shell: sesión y pestaña, nada tecleado.
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","agent":"shell"}"#,
        200,
    )
    .await;
    // Claude en `main` (sin CLAUDE_CONFIG_DIR) y en `relotto` (con ella y la
    // confianza heredada), Codex con ⚡ y Grok.
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","routeId":"claude:claude","harnessAccount":"main"}"#,
        200,
    )
    .await;
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f/","routeId":"claude:claude","harnessAccount":"relotto"}"#,
        200,
    )
    .await;
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","agent":"codex","danger":true}"#,
        200,
    )
    .await;
    // Grok: el modelo por omisión del registro no está en el `models_cache.json`
    // de la cuenta: el Python crea la sesión, la mata y responde 400.
    run(
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","routeId":"grok:grok"}"#,
        400,
    )
    .await;
    // Cuentas.
    run(
        "/account/add",
        r#"{"provider":"claude","alias":"relotto"}"#,
        409,
    )
    .await;
    run(
        "/account/add",
        r#"{"provider":"claude","alias":"main"}"#,
        409,
    )
    .await;
    run(
        "/account/add",
        r#"{"provider":"claude","alias":" nueva "}"#,
        200,
    )
    .await;
    run("/account/add", r#"{"provider":"codex"}"#, 200).await;
    run(
        "/account/add",
        r#"{"provider":"codex","alias":"dos","deviceAuth":true}"#,
        200,
    )
    .await;
    run(
        "/account/add",
        r#"{"provider":"grok","alias":"g1","cwd":"~"}"#,
        200,
    )
    .await;
    run("/account/add", r#"{"provider":"otro"}"#, 400).await;
    run("/account/add", r#"{"provider":"claude","alias":"-x"}"#, 400).await;
    // Lo tecleado (igual en los dos lados por `same_typed`).
    let has = |needle: &str| commands.iter().any(|c| c.contains(needle));
    assert!(
        has("CLAUDE_CONFIG_DIR=~/.claude-accounts/relotto claude"),
        "{commands:#?}"
    );
    let claude_main: Vec<&String> = commands
        .iter()
        .filter(|c| c.starts_with(" claude"))
        .collect();
    assert!(!claude_main.is_empty(), "{commands:#?}");
    assert!(
        claude_main.iter().all(|c| !c.contains("CLAUDE_CONFIG_DIR")),
        "main nunca lleva CLAUDE_CONFIG_DIR: {claude_main:?}"
    );
    assert!(has(" codex"), "{commands:#?}");
    assert!(
        has("--dangerously-bypass-approvals-and-sandbox"),
        "{commands:#?}"
    );
    assert!(has(
        "CLAUDE_CONFIG_DIR=~/.claude-accounts/nueva claude auth login --claudeai"
    ));
    assert!(has("CODEX_HOME=~/.codex-accounts/cuenta-2 codex -c"));
    assert!(has("login --device-auth"), "{commands:#?}");
    assert!(
        has("GROK_HOME=~/.grok-accounts/g1 grok login"),
        "{commands:#?}"
    );
    // El frente tecleó lanzadores cortos (y no el comando entero de respaldo).
    let launchers: Vec<String> =
        std::fs::read_dir(t.a.root.join(".local/share/comandos/launch"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        launchers.iter().any(|n| n.starts_with("claude-relotto"))
            && launchers.iter().any(|n| n.starts_with("codex"))
            && !launchers.iter().any(|n| n.contains("login")),
        "{launchers:?}"
    );
    // La configuración registrada y la confianza heredada, iguales.
    for table in ["usage_session_configs", "usage_changes"] {
        assert_eq!(usage_rows(&t.a, table), usage_rows(&t.b, table), "{table}");
    }
    assert_eq!(usage_rows(&t.a, "usage_session_configs").len(), 3);
    assert_eq!(usage_rows(&t.a, "usage_changes").len(), 1);
    let trust = home_file(&t.a, ".claude-accounts/relotto/.claude.json")
        .unwrap()
        .0;
    assert!(trust.contains("~/codebase/p2f"), "{trust}");
    let seeded = home_file(&t.a, ".claude-accounts/nueva/settings.json").unwrap();
    assert!(seeded.0.contains("cc-hook ñ") && seeded.0.contains("notifications_disabled"));
    assert_eq!(seeded.1, 0o600);
    assert_eq!(
        home_file(&t.a, ".codex-accounts/cuenta-2/config.toml")
            .unwrap()
            .1,
        0o600
    );
    let (sa, sb) = (sessions_of(&t.a), sessions_of(&t.b));
    assert_eq!(sa.len(), sb.len(), "{sa:?} {sb:?}");
}

#[tokio::test]
async fn session_new_bad_route_creates_no_session() {
    let Some(t) = Twin::start("snew-bad", |h| {
        seed_registry(h);
        support::tabs::seed_accounts(h);
    })
    .await
    else {
        return;
    };
    let run = t
        .post(
            "/session-new",
            r#"{"cwd":"~/codebase/p2f","routeId":"nadie:nada"}"#,
        )
        .await;
    run.assert_same();
    assert_eq!(run.front.status, 409);
    for side in [&t.a, &t.b] {
        let live = run_tmux(side, &["list-sessions", "-F", "#{session_name}"]);
        assert!(!live.contains("term-r"), "{live}");
    }
}

#[tokio::test]
async fn without_tmux_server_both_decline_before_effects() {
    if !support::tmux_available() {
        return;
    }
    let home = TestHome::new_short("snew-noserver");
    seed_registry(&home);
    let legacy = FakeLegacy::start().await;
    let fr = front(&home, legacy.port, options_for(&home)).await;
    for (path, body) in [
        (
            "/session-new",
            r#"{"cwd":"~/codebase/p2f","agent":"shell"}"#,
        ),
        ("/account/add", r#"{"provider":"grok","alias":"g1"}"#),
    ] {
        let wire = request_body(fr.port, "POST", path, "", body).await;
        assert_eq!(wire.text(), r#"{"legacy": true}"#, "{path}");
    }
    fr.stop().await;
    assert_eq!(legacy.requests().len(), 2);
    let socket = comandos_server::dash::native::tmux::private_socket(&home.tmux_dir());
    assert!(!socket.exists(), "nació un servidor tmux");
    assert!(!home.root.join(".grok-accounts").exists(), "ninguna cuenta");
    assert!(!support::tabs::exists(&home, "app-tab-open.json"));
}

/// Un HOME con espacio y comilla simple: las rutas de cuenta tecleadas van
/// con `shlex.quote` y el comando es el mismo byte a byte que el del Python.
#[tokio::test]
async fn hostile_home_path_is_quoted_like_python() {
    let Some(t) = Twin::start("snew q'a b", |h| {
        seed_registry(h);
        support::tabs::seed_accounts(h);
    })
    .await
    else {
        return;
    };
    assert_confined(&t);
    assert!(t.a.root.display().to_string().contains("q'a b"));
    let mut commands = Vec::new();
    for (path, body, want) in [
        (
            "/session-new",
            r#"{"cwd":"~/codebase/p2f","routeId":"claude:claude","harnessAccount":"relotto"}"#,
            200,
        ),
        ("/account/add", r#"{"provider":"codex"}"#, 200),
        (
            "/account/add",
            r#"{"provider":"claude","alias":"nueva"}"#,
            200,
        ),
    ] {
        let (status, log) = same_typed(&t, path, body).await;
        assert_eq!(status, want, "{path} {body}");
        commands.extend(typed(&log));
    }
    let has = |needle: &str| commands.iter().any(|c| c.contains(needle));
    assert!(
        has("CLAUDE_CONFIG_DIR='~/.claude-accounts/relotto' claude"),
        "{commands:#?}"
    );
    assert!(
        has("CODEX_HOME='~/.codex-accounts/cuenta-2' codex -c"),
        "{commands:#?}"
    );
    assert!(has(
        "CLAUDE_CONFIG_DIR='~/.claude-accounts/nueva' claude auth login"
    ));
    for table in ["usage_session_configs", "usage_changes"] {
        assert_eq!(usage_rows(&t.a, table), usage_rows(&t.b, table), "{table}");
    }
    let trust = home_file(&t.a, ".claude-accounts/relotto/.claude.json")
        .unwrap()
        .0;
    assert!(trust.contains("~/codebase/p2f"), "{trust}");
}

/// `/account/add` con `cwd` relativo. El Python lo mira con `os.path.isdir`
/// desde su directorio de trabajo, que en producción es el HOME (la unidad
/// `cc-dash-legacy` no fija `WorkingDirectory`): si no es carpeta ahí, cae al
/// HOME (el frente igual); si lo es, `tmux -c` recibiría una ruta relativa
/// cuya resolución depende del cliente tmux: el frente declina antes de nada.
#[tokio::test]
async fn account_add_relative_cwd_resolves_against_home_or_declines() {
    if !support::tmux_available() {
        return;
    }
    let home = TestHome::new_short("acct-rel");
    seed_registry(&home);
    support::tabs::seed_accounts(&home);
    let legacy = FakeLegacy::start().await;
    let mut opts = options_for(&home);
    // El directorio de trabajo del frente NO es el HOME: el relativo se mira
    // contra el HOME igual.
    opts.cwd = home.root.join("codebase");
    let fr = front(&home, legacy.port, opts).await;
    let wire = request_body(
        fr.port,
        "POST",
        "/account/add",
        "",
        r#"{"provider":"grok","alias":"g1","cwd":"codebase/p2f"}"#,
    )
    .await;
    assert_eq!(wire.text(), r#"{"legacy": true}"#);
    assert!(!home.root.join(".grok-accounts").exists(), "ninguna cuenta");
    // `p2f` existe bajo `opts.cwd`, no bajo el HOME: cae al HOME.
    let wire = request_body(
        fr.port,
        "POST",
        "/account/add",
        "",
        r#"{"provider":"grok","alias":"g2","cwd":"p2f"}"#,
    )
    .await;
    assert_eq!(wire.status, 200, "{}", wire.text());
    fr.stop().await;
    assert_eq!(legacy.requests().len(), 1);
    let log = support::tabs::TmuxLog(&home).read();
    let created: Vec<&Vec<String>> = log
        .iter()
        .filter(|args| args.first().is_some_and(|v| v == "new-session"))
        .filter(|args| args.iter().any(|a| a.starts_with("term-r")))
        .collect();
    assert_eq!(created.len(), 1, "{log:?}");
    assert_eq!(
        created[0].last().map(String::as_str),
        Some(home.root.display().to_string().as_str())
    );
}

#[tokio::test]
async fn session_new_additional_harnesses_match_python() {
    let t = Twin::start("snew-harness", |h| {
        seed_registry(h);
        support::tabs::seed_accounts(h);
    })
    .await
    .expect("confined harness twin");
    assert_confined(&t);
    for (route, account, command) in [
        ("acp:claude", "relotto", "cc-acp --agent claude"),
        ("opencode:opencode", "main", "opencode --model"),
        ("agy:agy", "main", "agy --model"),
    ] {
        let body = serde_json::json!({"cwd":"~/codebase/p2f","routeId":route,"motorAccount":account,"danger":true}).to_string();
        let (status, log) = same_typed(&t, "/session-new", &body).await;
        assert_eq!(status, 200, "{route}");
        let commands = typed(&log);
        assert!(commands.iter().any(|c| c.contains(command)), "{commands:?}");
        if route == "acp:claude" {
            assert!(
                commands
                    .iter()
                    .any(|c| c.contains("--account relotto") && c.contains("--danger")),
                "{commands:?}"
            );
        }
    }
    assert_eq!(
        usage_rows(&t.a, "usage_session_configs"),
        usage_rows(&t.b, "usage_session_configs")
    );
}

#[tokio::test]
async fn session_new_profile_matches_python_and_preserves_configuration() {
    let t = Twin::start("snew-profile", |h| {
        seed_registry(h);
        support::tabs::seed_accounts(h);
        let conn = comandos_store::usage::open_usage_db_at(&h.usage_db()).unwrap();
        comandos_store::session_profiles::save_profile(&conn, &serde_json::json!({"id":"p-native","name":"Privado","harness":"codex","routeId":"codex:codex"}),1700000000).unwrap();
        comandos_store::session_profiles::save_profile(&conn, &serde_json::json!({"id":"p-io","name":"IO","harness":"claude","routeId":"claude:claude","mcps":{"demo":false}}),1700000000).unwrap();
        let conf_path = h.root.join(".claude.json");
        let mut conf: serde_json::Value = serde_json::from_slice(&std::fs::read(&conf_path).unwrap()).unwrap();
        conf["mcpServers"] = serde_json::json!({"demo":{"command":"never-run"},"keep":{"command":"never-run-keep"}});
        std::fs::write(conf_path,conf.to_string()).unwrap();
        comandos_store::session_profiles::save_profile(&conn, &serde_json::json!({"id":"p-flags","name":"Flags","harness":"codex","routeId":"codex:codex","mcps":{"demo":false}}),1700000000).unwrap();
        std::fs::write(h.root.join(".codex/config.toml"), "# conservar literalmente\nmodel = 'modelo-del-usuario'\n[mcp_servers.demo]\ncommand = 'never-run'\n").unwrap();
    }).await.expect("confined profile twin");
    assert_confined(&t);
    let before: Vec<_> = [&t.a, &t.b]
        .iter()
        .map(|h| std::fs::read(h.root.join(".codex/config.toml")).unwrap())
        .collect();
    let missing = t
        .post(
            "/session-new",
            r#"{"profileId":"missing","cwd":"~/codebase/p2f"}"#,
        )
        .await;
    missing.assert_same();
    assert_eq!(missing.front.status, 409);
    let body = r#"{"cwd":"~/codebase/p2f","profileId":"p-native","danger":true}"#;
    let (status, log) = same_typed(&t, "/session-new", body).await;
    assert_eq!(status, 200);
    assert!(
        typed(&log)
            .iter()
            .any(|c| c.contains("COMANDOS_SESSION_PROFILE=p-native") && c.contains("codex")),
        "{log:?}"
    );
    let (status, log) = same_typed(
        &t,
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","profileId":"p-flags"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        typed(&log)
            .iter()
            .any(|c| c.contains("COMANDOS_SESSION_PROFILE=p-flags")
                && c.contains("mcp_servers.demo.enabled=false")),
        "{log:?}"
    );
    let (status, log) = same_typed(
        &t,
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","profileId":"p-io"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        typed(&log)
            .iter()
            .any(|c| c.contains("COMANDOS_SESSION_PROFILE=p-io")
                && c.contains("--strict-mcp-config --mcp-config")),
        "{log:?}"
    );
    for h in [&t.a, &t.b] {
        use std::os::unix::fs::PermissionsExt;
        let root = h.hooks().join("profile-launches");
        let files: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|f| f.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        let content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
        assert_eq!(
            content,
            serde_json::json!({"mcpServers":{"keep":{"command":"never-run-keep"}}})
        );
        assert_eq!(
            std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let original: serde_json::Value =
            serde_json::from_slice(&std::fs::read(h.root.join(".claude.json")).unwrap()).unwrap();
        assert!(
            original["mcpServers"]["demo"].is_object(),
            "global MCP config was changed"
        );
        std::fs::remove_dir_all(&root).unwrap();
        h.write("profile-launches", "occupied");
    }
    let (status, log) = same_logged(
        &t,
        "/session-new",
        r#"{"cwd":"~/codebase/p2f","profileId":"p-io"}"#,
    )
    .await;
    assert_eq!(status, 409);
    assert!(log.is_empty(), "IO failure created no session: {log:?}");
    for (i, h) in [&t.a, &t.b].iter().enumerate() {
        assert_eq!(
            std::fs::read(h.root.join(".codex/config.toml")).unwrap(),
            before[i]
        );
        let conn = rusqlite::Connection::open(h.usage_db()).unwrap();
        let profile =
            comandos_store::session_profiles::get_profile(&conn, &serde_json::json!("p-native"))
                .unwrap();
        assert_eq!(profile["routeId"], "codex:codex");
        assert_eq!(profile["updatedAt"], 1700000000);
    }
}
