//! `comandos-extensions hub` contra un catálogo falso con dos servidores stdio (un script
//! Python mínimo), uno roto y uno desactivado. HOME, catálogo y socket del broker son
//! temporales: nada toca la instalación real.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const FAKE: &str = r#"
import json, os, sys
name, log = sys.argv[1], sys.argv[2]
with open(log, 'a') as f:
    f.write(f'{name} {os.getpid()}\n')
tools = {
    'alpha': [
        {'name': 'send_message', 'description': 'Send a WhatsApp message to a chat',
         'inputSchema': {'type': 'object', 'properties': {'chat': {'type': 'string'}, 'text': {'type': 'string'}}, 'required': ['chat', 'text']}},
        {'name': 'list_chats', 'description': 'List recent chats', 'inputSchema': {'type': 'object'}},
    ],
    'beta': [
        {'name': 'create_task', 'description': 'Crear una tarea en ClickUp ' + 'x' * 400,
         'inputSchema': {'type': 'object', 'properties': {'title': {'type': 'string'}}}},
        {'name': 'explode', 'description': 'Always fails', 'inputSchema': {'type': 'object'}},
        {'name': 'soft_fail', 'description': 'Returns a tool error', 'inputSchema': {'type': 'object'}},
    ],
}[name]
for line in sys.stdin:
    m = json.loads(line)
    if 'id' not in m:
        continue
    method, p = m.get('method'), m.get('params') or {}
    out = {'jsonrpc': '2.0', 'id': m['id']}
    if method == 'initialize':
        out['result'] = {'protocolVersion': p.get('protocolVersion', '2025-03-26'), 'capabilities': {'tools': {}},
                         'serverInfo': {'name': name, 'version': '1'}, 'instructions': f'Servidor {name} de prueba. Más texto.'}
    elif method == 'tools/list':
        page = int(p.get('cursor') or 0)
        out['result'] = {'tools': tools[page:page + 1]}
        if page + 1 < len(tools):
            out['result']['nextCursor'] = str(page + 1)
    elif method == 'tools/call' and p['name'] == 'explode':
        out['error'] = {'code': -32000, 'message': 'kaboom'}
    elif method == 'tools/call' and p['name'] == 'soft_fail':
        out['result'] = {'content': [{'type': 'text', 'text': 'bad input'}], 'isError': True}
    elif method == 'tools/call':
        out['result'] = {'content': [{'type': 'text', 'text': json.dumps({'server': name, 'tool': p['name'], 'args': p.get('arguments')})}],
                         'structuredContent': {'pid': os.getpid()}}
    else:
        out['error'] = {'code': -32601, 'message': 'nope'}
    print(json.dumps(out), flush=True)
"#;

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "comandos-hub-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("run")).unwrap();
        let script = root.join("fake.py");
        std::fs::write(&script, FAKE).unwrap();
        let log = root.join("starts.log");
        let server = |name: &str| json!({"command":"/usr/bin/python3","args":["-I",script,name,log],"shared":false});
        let catalog = json!({"version":1,"servers":{
            "alpha":server("alpha"),
            "beta":server("beta"),
            // Registra su arranque y muere (no tiene herramientas en el script).
            "broken":server("broken"),
            "off":{"command":"/usr/bin/python3","args":["-c","raise SystemExit(9)"],"enabled":false},
        }});
        std::fs::write(root.join("catalog.json"), catalog.to_string()).unwrap();
        Self { root }
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn starts(&self, name: &str) -> Vec<i32> {
        std::fs::read_to_string(self.root.join("starts.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(n, _)| *n == name)
            .map(|(_, pid)| pid.parse().unwrap())
            .collect()
    }
    fn hub(&self) -> Hub {
        let mut child = Command::new(env!("CARGO_BIN_EXE_comandos-extensions"))
            .arg("--home")
            .arg(self.home())
            .arg("--catalog")
            .arg(self.root.join("catalog.json"))
            .arg("hub")
            .env("HOME", self.home())
            .env("XDG_RUNTIME_DIR", self.root.join("run"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Hub {
            stdin: Some(child.stdin.take().unwrap()),
            child,
            lines: rx,
            next: 0,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Hub {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    next: u64,
}
impl Hub {
    fn send(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(60))
                .expect("hub response");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["id"] == id {
                return value;
            }
        }
    }
    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name":tool,"arguments":arguments}));
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
    fn search(&mut self, arguments: Value) -> Value {
        let result = self.call("buscar_herramientas", arguments);
        assert!(result.get("isError").is_none(), "{result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
        );
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        response["result"].clone()
    }
    fn close(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        for _ in 0..200 {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        panic!("hub did not exit after stdin closed");
    }
}

fn gone(pid: i32) -> bool {
    for _ in 0..200 {
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
            Err(nix::errno::Errno::ESRCH) => return true,
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    false
}

fn names(result: &Value) -> Vec<String> {
    result["herramientas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            format!(
                "{}/{}",
                t["servidor"].as_str().unwrap(),
                t["herramienta"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn initialize_and_tools_list_expose_exactly_two_tools() {
    let fixture = Fixture::new();
    let mut hub = fixture.hub();
    let init = hub.initialize();
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "comandos");
    assert!(init["capabilities"]["tools"].is_object());
    let instructions = init["instructions"].as_str().unwrap();
    assert!(instructions.len() < 2048, "{instructions}");
    for name in ["alpha", "beta", "broken"] {
        assert!(
            instructions.contains(&format!("- {name}")),
            "{instructions}"
        );
    }
    assert!(
        !instructions.contains("- off"),
        "disabled servers are not offered"
    );
    assert!(instructions.contains("buscar_herramientas"));
    let tools = hub.request("tools/list", json!({}))["result"]["tools"].clone();
    let listed: Vec<_> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, ["buscar_herramientas", "usar_herramienta"]);
    for tool in tools.as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert!(tool["description"].as_str().unwrap().len() > 40);
    }
    assert_eq!(hub.request("ping", json!({}))["result"], json!({}));
    assert_eq!(
        hub.request("unknown/method", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        hub.request("tools/call", json!({"name":"nope","arguments":{}}))["error"]["code"],
        -32602
    );
    // Nada se indexa ni se arranca hasta la primera búsqueda.
    assert!(fixture.starts("alpha").is_empty());
    assert!(hub.close().success());
}

#[test]
fn search_indexes_lazily_ranks_and_reports_unavailable_servers() {
    let fixture = Fixture::new();
    let mut hub = fixture.hub();
    hub.initialize();
    let overview = hub.search(json!({"query":""}));
    let servers: Vec<_> = overview["servidores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["servidor"].as_str().unwrap())
        .collect();
    assert_eq!(servers, ["alpha", "beta", "broken"]);
    assert!(
        fixture.starts("alpha").is_empty(),
        "overview does not index"
    );

    let found = hub.search(json!({"query":"whatsapp message"}));
    assert_eq!(names(&found)[0], "alpha/send_message");
    let first = &found["herramientas"][0];
    assert_eq!(first["inputSchema"]["required"], json!(["chat", "text"]));
    assert_eq!(first["descripcion"], "Send a WhatsApp message to a chat");
    let unavailable = found["no_disponibles"].as_array().unwrap();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0]["servidor"], "broken");
    assert!(
        unavailable[0]["motivo"]
            .as_str()
            .unwrap()
            .starts_with("no disponible"),
        "{unavailable:?}"
    );
    // Consulta en español y sin tildes; la descripción larga se recorta.
    let task = hub.search(json!({"query":"crear tárea","limite":1}));
    assert_eq!(names(&task), ["beta/create_task"]);
    assert!(
        task["herramientas"][0]["descripcion"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 300
    );
    // Query vacía + servidor: lista sus herramientas (paginadas en el servidor).
    let beta = hub.search(json!({"query":"","servidor":"beta"}));
    assert_eq!(beta["total"], 3);
    assert_eq!(
        names(&beta),
        ["beta/create_task", "beta/explode", "beta/soft_fail"]
    );
    assert!(beta["herramientas"][0]["inputSchema"].is_object());
    let compact = hub.search(json!({"query":"","servidor":"beta","limite":2}));
    assert!(compact["herramientas"][0].get("inputSchema").is_none());
    assert!(compact["nota"].is_string());
    let unknown = hub.call("buscar_herramientas", json!({"query":"x","servidor":"off"}));
    assert_eq!(unknown["isError"], true);
    // El índice se hizo una sola vez por servidor y quedó en disco.
    assert_eq!(fixture.starts("alpha").len(), 1);
    assert_eq!(fixture.starts("beta").len(), 1);
    assert_eq!(
        fixture.starts("broken").len(),
        1,
        "a failure is not retried at once"
    );
    let cache = fixture
        .home()
        .join(".local/state/comandos/extensions/hub/alpha.json");
    let cached: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(cached["herramientas"].as_array().unwrap().len(), 2);
    assert_eq!(cached["resumen"], "Servidor alpha de prueba");
    assert!(hub.close().success());
    for pid in fixture
        .starts("alpha")
        .into_iter()
        .chain(fixture.starts("beta"))
    {
        assert!(
            gone(pid),
            "indexing child {pid} must not outlive its listing"
        );
    }

    // Otro hub reutiliza el índice en disco sin arrancar los servidores.
    let mut again = fixture.hub();
    let init = again.initialize();
    assert!(
        init["instructions"]
            .as_str()
            .unwrap()
            .contains("- alpha: Servidor alpha de prueba")
    );
    assert_eq!(
        names(&again.search(json!({"query":"send_message"})))[0],
        "alpha/send_message"
    );
    assert_eq!(fixture.starts("alpha").len(), 1);
    // El fallo de `broken` también quedó en disco: no se reintenta en cada sesión.
    let failed = again.search(json!({"query":"x","servidor":"broken"}));
    assert_eq!(failed["no_disponibles"][0]["servidor"], "broken");
    assert_eq!(fixture.starts("broken").len(), 1);
    assert!(again.close().success());
}

#[test]
fn call_round_trips_reuses_one_child_and_reports_errors_as_tool_errors() {
    let fixture = Fixture::new();
    let mut hub = fixture.hub();
    hub.initialize();
    let result = hub.call(
        "usar_herramienta",
        json!({"servidor":"alpha","herramienta":"send_message","argumentos":{"chat":"c1","text":"hola"}}),
    );
    assert!(result.get("isError").is_none(), "{result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        text,
        json!({"server":"alpha","tool":"send_message","args":{"chat":"c1","text":"hola"}})
    );
    let pid = result["structuredContent"]["pid"].as_i64().unwrap();
    // Argumentos como texto JSON, y omitidos.
    let second = hub.call(
        "usar_herramienta",
        json!({"servidor":"alpha","herramienta":"list_chats","argumentos":"{\"n\":1}"}),
    );
    assert_eq!(second["structuredContent"]["pid"].as_i64().unwrap(), pid);
    let third = hub.call(
        "usar_herramienta",
        json!({"servidor":"alpha","herramienta":"list_chats"}),
    );
    assert_eq!(third["structuredContent"]["pid"].as_i64().unwrap(), pid);
    assert_eq!(fixture.starts("alpha").len(), 1, "one persistent child");
    // Una búsqueda posterior reutiliza el hijo vivo en lugar de arrancar otro.
    hub.search(json!({"query":"chats","servidor":"alpha"}));
    assert_eq!(fixture.starts("alpha").len(), 1);

    let rpc_error = hub.call(
        "usar_herramienta",
        json!({"servidor":"beta","herramienta":"explode","argumentos":{}}),
    );
    assert_eq!(rpc_error["isError"], true);
    assert!(
        rpc_error["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("kaboom")
    );
    let soft = hub.call(
        "usar_herramienta",
        json!({"servidor":"beta","herramienta":"soft_fail","argumentos":{}}),
    );
    assert_eq!(
        soft,
        json!({"content":[{"type":"text","text":"bad input"}],"isError":true})
    );
    let broken = hub.call(
        "usar_herramienta",
        json!({"servidor":"broken","herramienta":"x","argumentos":{}}),
    );
    assert_eq!(broken["isError"], true);
    let disabled = hub.call(
        "usar_herramienta",
        json!({"servidor":"off","herramienta":"x","argumentos":{}}),
    );
    assert_eq!(disabled["isError"], true);
    let invalid = hub.call(
        "usar_herramienta",
        json!({"servidor":"alpha","herramienta":"x","argumentos":[1]}),
    );
    assert_eq!(invalid["isError"], true);
    // El hub sigue vivo tras los errores.
    assert_eq!(
        hub.call(
            "usar_herramienta",
            json!({"servidor":"alpha","herramienta":"list_chats"})
        )["structuredContent"]["pid"]
            .as_i64()
            .unwrap(),
        pid
    );
    assert!(hub.close().success());
    for pid in fixture
        .starts("alpha")
        .into_iter()
        .chain(fixture.starts("beta"))
    {
        assert!(gone(pid), "child {pid} must die with the hub");
    }
}
