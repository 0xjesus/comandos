//! `hub`: servidor MCP stdio con solo dos herramientas que da acceso bajo demanda a todo
//! el catálogo compartido. OpenCode y agy cargan las definiciones de cada MCP en cada
//! petición y no saben diferirlas (≈1.100 herramientas, ≈500k tokens por mensaje); con el
//! hub pagan dos herramientas y cargan el resto solo cuando el agente las busca.
//!
//! - `buscar_herramientas` consulta un índice de `tools/list` por servidor, guardado en
//!   `state_dir/hub/<servidor>.json`. Se construye la primera vez que hace falta, lanzando
//!   `serve <servidor>` en paralelo (con tope de tiempo por servidor), y se refresca en
//!   segundo plano pasadas 24 h o cuando cambia la entrada del catálogo.
//! - `usar_herramienta` reenvía `tools/call` a un hijo `serve <servidor>` que se arranca
//!   la primera vez, se reutiliza mientras viva el hub y muere con él.
//!
//! Los hijos son el mismo `serve` que reciben los demás clientes: broker compartido,
//! credenciales y filtros `enabled_tools`/`disabled_tools` incluidos.
use crate::{
    Result, config,
    serve::normalize,
    transport::{MAX_INFLIGHT, Transport, line},
};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{OnceCell, Semaphore, mpsc, watch},
    task::{AbortHandle, JoinSet},
};

/// Nombre con el que los clientes declaran el hub.
pub const NAME: &str = "comandos";
pub const SEARCH: &str = "buscar_herramientas";
pub const CALL: &str = "usar_herramienta";
const TTL: f64 = 24.0 * 3600.0;
const LIST_TIMEOUT: Duration = Duration::from_secs(20);
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// Una búsqueda espera al índice como mucho esto; lo que falte sale como «indexando».
const SEARCH_WAIT: Duration = Duration::from_secs(25);
/// Un servidor que no listó no se reintenta antes de esto, tampoco desde otra sesión:
/// así un servidor caído no cuesta `LIST_TIMEOUT` en la primera búsqueda de cada panel.
const RETRY_AFTER: f64 = 600.0;
const CONCURRENCY: usize = 8;
const DESCRIPTION: usize = 300;
const SUMMARY: usize = 70;
const INSTRUCTIONS: usize = 2000;
const DEFAULT_LIMIT: usize = 15;
const MAX_LIMIT: usize = 50;
const MAX_PAGES: usize = 30;

fn tool_definitions() -> Value {
    json!([
        {
            "name": SEARCH,
            "title": "Buscar herramientas de ComandOS",
            "description": "Busca herramientas en los servidores MCP de ComandOS, que no están cargados en esta sesión para ahorrar contexto. Devuelve servidor, herramienta, descripción e inputSchema de las que coinciden. Úsala siempre antes de usar_herramienta. Con query vacía y servidor, lista las herramientas de ese servidor; sin query ni servidor, lista los servidores disponibles.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Palabras clave: acción, tema o nombre de herramienta (p. ej. «enviar correo», «whatsapp mensajes», «crear tarea clickup»)."},
                    "servidor": {"type": "string", "description": "Opcional: limita la búsqueda a este servidor."},
                    "limite": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "description": "Opcional: máximo de resultados (15 por omisión)."}
                },
                "required": ["query"]
            }
        },
        {
            "name": CALL,
            "title": "Usar una herramienta de ComandOS",
            "description": "Ejecuta una herramienta de un servidor MCP de ComandOS encontrada con buscar_herramientas. Pasa el servidor, el nombre exacto de la herramienta y los argumentos que pide su inputSchema; devuelve el resultado de la herramienta tal cual.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "servidor": {"type": "string", "description": "Servidor que devolvió buscar_herramientas."},
                    "herramienta": {"type": "string", "description": "Nombre exacto de la herramienta."},
                    "argumentos": {"type": "object", "additionalProperties": true, "description": "Argumentos según el inputSchema de la herramienta."}
                },
                "required": ["servidor", "herramienta"]
            }
        }
    ])
}

#[derive(Clone)]
struct Listing {
    updated: f64,
    fingerprint: String,
    summary: String,
    tools: Arc<Vec<Value>>,
}
#[derive(Clone)]
struct Failure {
    reason: String,
    at: f64,
    fingerprint: String,
}
/// Estado de un servidor en el índice: su último listado (aunque caduque) y su último fallo.
#[derive(Clone, Default)]
struct Entry {
    listing: Option<Listing>,
    failure: Option<Failure>,
}
impl Entry {
    fn to_json(&self, name: &str) -> Value {
        let mut data = json!({"version":1,"servidor":name});
        if let Some(l) = &self.listing {
            data["actualizado"] = json!(l.updated);
            data["huella"] = json!(l.fingerprint);
            data["resumen"] = json!(l.summary);
            data["herramientas"] = json!(*l.tools);
        }
        if let Some(f) = &self.failure {
            data["fallo"] = json!({"motivo":f.reason,"en":f.at,"huella":f.fingerprint});
        }
        data
    }
    fn from_json(data: &Value) -> Option<Self> {
        let listing = (|| {
            Some(Listing {
                updated: data["actualizado"].as_f64()?,
                fingerprint: data["huella"].as_str()?.to_owned(),
                summary: data["resumen"].as_str().unwrap_or("").to_owned(),
                tools: Arc::new(data["herramientas"].as_array()?.clone()),
            })
        })();
        let failure = (|| {
            let f = &data["fallo"];
            Some(Failure {
                reason: f["motivo"].as_str()?.to_owned(),
                at: f["en"].as_f64()?,
                fingerprint: f["huella"].as_str()?.to_owned(),
            })
        })();
        (listing.is_some() || failure.is_some()).then_some(Self { listing, failure })
    }
    fn failed_recently(&self, fingerprint: &str, now: f64) -> bool {
        self.failure
            .as_ref()
            .is_some_and(|f| f.fingerprint == fingerprint && now - f.at < RETRY_AFTER)
    }
}
struct Child {
    transport: Transport,
    summary: String,
}

struct Hub {
    home: PathBuf,
    catalog: PathBuf,
    program: String,
    prefix: Vec<String>,
    index: Mutex<HashMap<String, Entry>>,
    running: Mutex<HashMap<String, watch::Receiver<bool>>>,
    loaded: OnceCell<()>,
    permits: Arc<Semaphore>,
    children: tokio::sync::Mutex<HashMap<String, Arc<OnceCell<Arc<Child>>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
/// Texto en una línea, sin espacios repetidos y cortado en `limit` caracteres.
fn clip(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        return flat;
    }
    let mut out: String = flat.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}
/// Minúsculas y sin tildes, para comparar consultas en español con nombres en inglés.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}
fn words(text: &str) -> Vec<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}
fn text_result(value: &Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}]})
}
fn tool_error(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}
fn summary_of(initialized: &Value) -> String {
    let title = initialized["serverInfo"]["title"].as_str().unwrap_or("");
    let text = if title.is_empty() {
        initialized["instructions"]
            .as_str()
            .and_then(|i| i.lines().map(str::trim).find(|l| !l.is_empty()))
            .unwrap_or("")
    } else {
        title
    };
    let sentence = text.split(". ").next().unwrap_or(text);
    clip(sentence.trim_start_matches('#').trim(), SUMMARY)
}
fn compact_tool(tool: &Value) -> Option<Value> {
    let name = tool["name"].as_str().filter(|n| !n.is_empty())?;
    let mut out = json!({"name":name,"description":tool["description"].as_str().unwrap_or(""),"inputSchema":tool.get("inputSchema").filter(|s| s.is_object()).cloned().unwrap_or_else(|| json!({"type":"object"}))});
    if let Some(title) = tool["title"].as_str().filter(|t| !t.is_empty()) {
        out["title"] = json!(title);
    }
    Some(out)
}

async fn rpc(transport: &Transport, method: &str, params: Value) -> Result<Value> {
    let response = transport
        .request(json!({"jsonrpc":"2.0","method":method,"params":params}))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(clip(
            error["message"].as_str().unwrap_or("error del servidor"),
            200,
        ));
    }
    response
        .get("result")
        .filter(|r| r.is_object())
        .cloned()
        .ok_or_else(|| "respuesta inválida del servidor".into())
}
async fn list_tools(transport: &Transport) -> Result<Vec<Value>> {
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let params = cursor
            .as_ref()
            .map(|c| json!({"cursor":c}))
            .unwrap_or_else(|| json!({}));
        let page = rpc(transport, "tools/list", params).await?;
        let items = page["tools"]
            .as_array()
            .ok_or("tools/list sin lista de herramientas")?;
        tools.extend(items.iter().filter_map(compact_tool));
        cursor = page["nextCursor"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(str::to_owned);
        if cursor.is_none() {
            return Ok(tools);
        }
    }
    Ok(tools)
}

impl Hub {
    fn new(home: &Path, catalog: &Path) -> Result<Self> {
        let executable = std::env::current_exe().map_err(|_| "Extension executable unavailable")?;
        let mut prefix = Vec::new();
        // El binario multicall (`comandos`) despacha primero `ext`; el autónomo no.
        if executable
            .file_name()
            .is_none_or(|name| name != "comandos-extensions")
        {
            prefix.push("ext".to_owned());
        }
        prefix.extend([
            "--home".into(),
            home.to_string_lossy().into_owned(),
            "--catalog".into(),
            catalog.to_string_lossy().into_owned(),
        ]);
        Ok(Self {
            home: home.into(),
            catalog: catalog.into(),
            program: executable
                .to_str()
                .ok_or("Extension executable unavailable")?
                .into(),
            prefix,
            index: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            loaded: OnceCell::new(),
            permits: Arc::new(Semaphore::new(CONCURRENCY)),
            children: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Servidores habilitados del catálogo, leídos en cada uso: un cambio de catálogo se ve
    /// sin reiniciar el hub. Mismo criterio que `serve`.
    fn servers(&self) -> Result<Map<String, Value>> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&self.catalog)
            .map_err(|_| "Catalog unavailable")?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Catalog unavailable")?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Catalog too large".into());
        }
        let mut catalog = config::parse_json(&bytes)?;
        if catalog["version"] != 1 {
            return Err("Unsupported catalog".into());
        }
        let servers = catalog["servers"]
            .as_object_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        Ok(servers
            .into_iter()
            .filter(|(name, spec)| {
                name != NAME
                    && spec.as_object().is_some_and(|o| !o.is_empty())
                    && spec.get("enabled").is_none_or(crate::py_truthy)
            })
            .collect())
    }

    fn cache_path(&self, name: &str) -> PathBuf {
        config::state_dir(&self.home)
            .join("hub")
            .join(format!("{name}.json"))
    }

    /// Carga una vez el índice guardado en disco para los servidores del catálogo.
    async fn load_disk(self: &Arc<Self>) {
        self.loaded
            .get_or_init(|| async {
                let Ok(servers) = self.servers() else { return };
                let names: Vec<String> = servers.keys().filter(|n| safe(n)).cloned().collect();
                let hub = self.clone();
                let loaded = tokio::task::spawn_blocking(move || {
                    names
                        .into_iter()
                        .filter_map(|name| {
                            let raw = std::fs::read(hub.cache_path(&name)).ok()?;
                            let data: Value = serde_json::from_slice(&raw).ok()?;
                            Some((name, Entry::from_json(&data)?))
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                let mut index = lock(&self.index);
                for (name, entry) in loaded {
                    index.entry(name).or_insert(entry);
                }
            })
            .await;
    }

    fn child_spec(&self, name: &str) -> Value {
        let mut args = self.prefix.clone();
        args.extend(["serve".to_owned(), name.to_owned()]);
        json!({"command":self.program,"args":args})
    }

    async fn connect(&self, name: &str) -> Result<Child> {
        let transport = Transport::connect(&self.home, name, &self.child_spec(name))
            .await
            .map_err(|e| e.to_string())?;
        let initialized = rpc(&transport, "initialize", json!({"protocolVersion":normalize::LATEST_PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"comandos-hub","version":env!("CARGO_PKG_VERSION")}})).await?;
        transport
            .notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await
            .map_err(|e| e.to_string())?;
        Ok(Child {
            transport,
            summary: summary_of(&initialized),
        })
    }

    /// Hijo persistente para `usar_herramienta`: se arranca una vez y se reutiliza.
    async fn child(&self, name: &str) -> Result<Arc<Child>> {
        for _ in 0..2 {
            let cell = self
                .children
                .lock()
                .await
                .entry(name.to_owned())
                .or_default()
                .clone();
            let result = cell
                .get_or_try_init(|| async {
                    tokio::time::timeout(START_TIMEOUT, self.connect(name))
                        .await
                        .map_err(|_| "tiempo de arranque agotado".to_owned())?
                        .map(Arc::new)
                })
                .await
                .cloned();
            let stale = match &result {
                Ok(child) => *child.transport.closed.borrow(),
                Err(_) => true,
            };
            if stale {
                let mut children = self.children.lock().await;
                if children.get(name).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                    children.remove(name);
                }
            }
            match result {
                Ok(child) if !stale => return Ok(child),
                Ok(_) => continue,
                Err(error) => return Err(error),
            }
        }
        Err("el servidor se cerró".into())
    }

    async fn live_child(&self, name: &str) -> Option<Arc<Child>> {
        let children = self.children.lock().await;
        children
            .get(name)
            .and_then(|cell| cell.get().cloned())
            .filter(|child| !*child.transport.closed.borrow())
    }

    async fn index_one(self: Arc<Self>, name: String, fingerprint: String) {
        let _permit = self.permits.clone().acquire_owned().await;
        let outcome = tokio::time::timeout(LIST_TIMEOUT, async {
            if let Some(child) = self.live_child(&name).await {
                return list_tools(&child.transport)
                    .await
                    .map(|tools| (tools, child.summary.clone()));
            }
            let mut child = self.connect(&name).await?;
            let tools = list_tools(&child.transport).await;
            child.transport.shutdown().await;
            tools.map(|tools| (tools, child.summary))
        })
        .await
        .unwrap_or_else(|_| Err("no respondió a tiempo".into()));
        let entry = {
            let mut index = lock(&self.index);
            let entry = index.entry(name.clone()).or_default();
            match outcome {
                Ok((tools, summary)) => {
                    entry.listing = Some(Listing {
                        updated: crate::metadata::now(),
                        fingerprint,
                        summary,
                        tools: Arc::new(tools),
                    });
                    entry.failure = None;
                }
                // Un refresco fallido conserva el listado anterior.
                Err(reason) => {
                    entry.failure = Some(Failure {
                        reason: clip(&reason, 200),
                        at: crate::metadata::now(),
                        fingerprint,
                    });
                }
            }
            entry.clone()
        };
        if safe(&name) {
            let path = self.cache_path(&name);
            let data = entry.to_json(&name);
            // Un fallo de disco no invalida el índice en memoria.
            let _ = tokio::task::spawn_blocking(move || config::save_json(&path, &data)).await;
        }
        lock(&self.running).remove(&name);
    }

    fn start(self: &Arc<Self>, name: &str, fingerprint: String) -> watch::Receiver<bool> {
        let mut running = lock(&self.running);
        if let Some(receiver) = running.get(name) {
            return receiver.clone();
        }
        let (done, receiver) = watch::channel(false);
        running.insert(name.to_owned(), receiver.clone());
        let hub = self.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            hub.index_one(name, fingerprint).await;
            let _ = done.send(true);
        });
        receiver
    }

    /// Deja indexados `servers`: espera a los que faltan (hasta `SEARCH_WAIT`) y refresca
    /// en segundo plano los caducados.
    async fn ensure(self: &Arc<Self>, servers: &[(&String, &Value)]) {
        self.load_disk().await;
        let mut waits = Vec::new();
        for (name, spec) in servers {
            let Ok(fingerprint) = config::fingerprint(spec) else {
                continue;
            };
            let now = crate::metadata::now();
            let entry = lock(&self.index).get(*name).cloned().unwrap_or_default();
            let current = entry
                .listing
                .as_ref()
                .filter(|l| l.fingerprint == fingerprint);
            let (start, wait) = if entry.failed_recently(&fingerprint, now) {
                (false, false)
            } else if let Some(listing) = current {
                (now - listing.updated > TTL, false)
            } else {
                (true, true)
            };
            if start {
                let mut receiver = self.start(name, fingerprint);
                if wait {
                    waits.push(async move {
                        let _ = receiver.wait_for(|done| *done).await;
                    });
                }
            }
        }
        let _ = tokio::time::timeout(SEARCH_WAIT, futures_util::future::join_all(waits)).await;
    }

    fn unavailable(&self, name: &str) -> Option<Value> {
        let entry = lock(&self.index).get(name).cloned().unwrap_or_default();
        let reason = match (&entry.listing, &entry.failure) {
            (Some(_), _) => return None,
            (None, Some(failure)) => format!("no disponible: {}", failure.reason),
            (None, None) if lock(&self.running).contains_key(name) => {
                "indexando; reintenta en unos segundos".to_owned()
            }
            (None, None) => "sin índice".to_owned(),
        };
        Some(json!({"servidor":name,"motivo":reason}))
    }

    async fn instructions(self: &Arc<Self>) -> String {
        self.load_disk().await;
        let servers = self.servers().unwrap_or_default();
        let header = format!(
            "ComandOS da acceso bajo demanda a {} servidores MCP sin cargar sus herramientas en el contexto. Para usarlos: llama a {SEARCH} con palabras clave (y opcionalmente servidor) y después a {CALL} con servidor, herramienta y argumentos según el inputSchema devuelto.\nServidores:",
            servers.len()
        );
        let mut entries: Vec<(String, String)> = {
            let index = lock(&self.index);
            servers
                .iter()
                .map(|(name, spec)| {
                    let purpose = spec["description"]
                        .as_str()
                        .map(|d| clip(d, SUMMARY))
                        .filter(|d| !d.is_empty())
                        .or_else(|| match index.get(name) {
                            Some(Entry {
                                listing: Some(l), ..
                            }) if !l.summary.is_empty() => Some(l.summary.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    (name.clone(), purpose)
                })
                .collect()
        };
        entries.sort();
        let render = |with_purpose: bool| {
            let mut text = header.clone();
            for (name, purpose) in &entries {
                text.push_str("\n- ");
                text.push_str(name);
                if with_purpose && !purpose.is_empty() {
                    text.push_str(": ");
                    text.push_str(purpose);
                }
            }
            text
        };
        let full = render(true);
        if full.len() <= INSTRUCTIONS {
            return full;
        }
        let names = render(false);
        if names.len() <= INSTRUCTIONS {
            return names;
        }
        let mut text = header.clone();
        let mut shown = 0;
        for (name, _) in &entries {
            if text.len() + name.len() + 40 > INSTRUCTIONS {
                break;
            }
            text.push_str("\n- ");
            text.push_str(name);
            shown += 1;
        }
        text.push_str(&format!(
            "\n… y {} más (llama a {SEARCH} sin query para verlos).",
            entries.len() - shown
        ));
        text
    }

    async fn search(self: &Arc<Self>, args: &Value) -> Value {
        let query = args["query"].as_str().unwrap_or("").trim().to_owned();
        let server = args["servidor"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| crate::catalog::alias(s).to_owned());
        let limit = args["limite"]
            .as_u64()
            .or_else(|| args["limite"].as_f64().map(|f| f.max(0.0) as u64))
            .map(|n| (n as usize).clamp(1, MAX_LIMIT));
        let servers = match self.servers() {
            Ok(servers) => servers,
            Err(error) => return tool_error(&format!("Catálogo no disponible: {error}")),
        };
        if let Some(server) = &server
            && !servers.contains_key(server)
        {
            return tool_error(&format!(
                "Servidor desconocido o desactivado: {server}. Disponibles: {}",
                servers.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if query.is_empty() && server.is_none() {
            self.load_disk().await;
            let index = lock(&self.index);
            let mut list: Vec<Value> = servers
                .iter()
                .map(|(name, spec)| {
                    let mut row = json!({"servidor":name});
                    if let Some(d) = spec["description"].as_str() {
                        row["descripcion"] = json!(clip(d, DESCRIPTION));
                    }
                    if let Some(Entry {
                        listing: Some(l), ..
                    }) = index.get(name)
                    {
                        if !l.summary.is_empty() && row.get("descripcion").is_none() {
                            row["descripcion"] = json!(l.summary);
                        }
                        row["herramientas"] = json!(l.tools.len());
                    }
                    row
                })
                .collect();
            list.sort_by(|a, b| a["servidor"].as_str().cmp(&b["servidor"].as_str()));
            return text_result(
                &json!({"servidores":list,"siguiente_paso":format!("Llama a {SEARCH} con una query o con servidor para ver sus herramientas.")}),
            );
        }
        let targets: Vec<(&String, &Value)> = servers
            .iter()
            .filter(|(name, _)| server.as_ref().is_none_or(|s| s == *name))
            .collect();
        self.ensure(&targets).await;
        let mut unavailable = Vec::new();
        let mut listings = Vec::new();
        {
            let index = lock(&self.index);
            for (name, _) in &targets {
                if let Some(Entry {
                    listing: Some(listing),
                    ..
                }) = index.get(*name)
                {
                    listings.push(((*name).clone(), listing.tools.clone()));
                }
            }
        }
        for (name, _) in &targets {
            if let Some(row) = self.unavailable(name) {
                unavailable.push(row);
            }
        }
        let entry = |server: &str, tool: &Value, schema: bool| {
            let mut row = json!({"servidor":server,"herramienta":tool["name"],"descripcion":clip(tool["description"].as_str().unwrap_or(""),DESCRIPTION)});
            if schema {
                row["inputSchema"] = tool["inputSchema"].clone();
            }
            row
        };
        let mut result = if query.is_empty() {
            // Listado de un servidor: todas sus herramientas; el esquema solo si caben.
            let (name, tools) = listings
                .first()
                .map(|(n, t)| (n.clone(), t.clone()))
                .unwrap_or_else(|| (server.clone().unwrap_or_default(), Arc::new(Vec::new())));
            let limit = limit.unwrap_or(DEFAULT_LIMIT);
            let schema = tools.len() <= limit;
            let mut result = json!({"servidor":name,"total":tools.len(),"herramientas":tools.iter().map(|t| entry(&name,t,schema)).collect::<Vec<_>>()});
            if !schema {
                result["nota"] = json!(format!(
                    "Sin inputSchema por ser {} herramientas: busca por nombre con {SEARCH} para ver el esquema.",
                    tools.len()
                ));
            }
            result
        } else {
            let limit = limit.unwrap_or(DEFAULT_LIMIT);
            let tokens = words(&query);
            let exact = fold(&query);
            let mut scored = Vec::new();
            for (server, tools) in &listings {
                let server_words = words(server);
                let folded_server = fold(server);
                for tool in tools.iter() {
                    let name = tool["name"].as_str().unwrap_or("");
                    let folded_name = fold(name);
                    let name_words = words(name);
                    let title = fold(tool["title"].as_str().unwrap_or(""));
                    let description = fold(tool["description"].as_str().unwrap_or(""));
                    let description_words = words(&description);
                    let mut matched = 0;
                    let mut score = 0;
                    for token in &tokens {
                        let mut hit = 0;
                        if name_words.contains(token) {
                            hit += 6;
                        } else if folded_name.contains(token.as_str())
                            || title.contains(token.as_str())
                        {
                            hit += 4;
                        }
                        if server_words.contains(token) {
                            hit += 3;
                        } else if folded_server.contains(token.as_str()) {
                            hit += 2;
                        }
                        if description_words.contains(token) {
                            hit += 2;
                        } else if token.len() > 2 && description.contains(token.as_str()) {
                            hit += 1;
                        }
                        if hit > 0 {
                            matched += 1;
                            score += hit;
                        }
                    }
                    if folded_name == exact || format!("{folded_server} {folded_name}") == exact {
                        matched += 1;
                        score += 50;
                    }
                    if matched > 0 {
                        scored.push((matched * 100 + score, server.clone(), tool));
                    }
                }
            }
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2["name"].as_str().cmp(&b.2["name"].as_str()))
            });
            let total = scored.len();
            let hits: Vec<Value> = scored
                .into_iter()
                .take(limit)
                .map(|(_, server, tool)| entry(&server, tool, true))
                .collect();
            let mut result = json!({"total":total,"herramientas":hits});
            if total == 0 {
                result["nota"] = json!(format!(
                    "Sin coincidencias. Prueba otras palabras (en inglés suelen estar los nombres) o {SEARCH} sin query para ver los servidores."
                ));
            }
            result
        };
        if !unavailable.is_empty() {
            result["no_disponibles"] = json!(unavailable);
        }
        result["siguiente_paso"] = json!(format!(
            "Llama a {CALL} con servidor, herramienta y argumentos."
        ));
        text_result(&result)
    }

    async fn call(self: &Arc<Self>, args: &Value) -> Value {
        let (Some(server), Some(tool)) = (
            args["servidor"].as_str().filter(|s| !s.is_empty()),
            args["herramienta"].as_str().filter(|s| !s.is_empty()),
        ) else {
            return tool_error("Faltan servidor y herramienta (usa buscar_herramientas).");
        };
        let server = crate::catalog::alias(server);
        let arguments = match args.get("argumentos") {
            None | Some(Value::Null) => json!({}),
            Some(Value::Object(o)) => Value::Object(o.clone()),
            // Algunos modelos mandan los argumentos como texto JSON.
            Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
                Ok(v) if v.is_object() => v,
                _ => return tool_error("argumentos debe ser un objeto JSON."),
            },
            Some(_) => return tool_error("argumentos debe ser un objeto JSON."),
        };
        match self.servers() {
            Ok(servers) if servers.contains_key(server) => {}
            Ok(_) => {
                return tool_error(&format!(
                    "Servidor desconocido o desactivado: {server}. Usa buscar_herramientas sin query para ver los disponibles."
                ));
            }
            Err(error) => return tool_error(&format!("Catálogo no disponible: {error}")),
        }
        let child = match self.child(server).await {
            Ok(child) => child,
            Err(error) => return tool_error(&format!("No se pudo iniciar {server}: {error}")),
        };
        let response = child
            .transport
            .request(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":tool,"arguments":arguments}}))
            .await;
        match response {
            Ok(response) => {
                if let Some(error) = response.get("error") {
                    return tool_error(&format!(
                        "{server}/{tool} falló: {}",
                        error["message"].as_str().unwrap_or("error del servidor")
                    ));
                }
                match response.get("result").filter(|r| r.is_object()) {
                    Some(result) => result.clone(),
                    None => tool_error(&format!("{server}/{tool}: respuesta inválida.")),
                }
            }
            Err(error) => tool_error(&format!("{server}/{tool} falló: {error}")),
        }
    }

    async fn close(&self) {
        let children = std::mem::take(&mut *self.children.lock().await);
        for cell in children.into_values() {
            if let Ok(cell) = Arc::try_unwrap(cell)
                && let Some(child) = cell.into_inner()
                && let Ok(mut child) = Arc::try_unwrap(child)
            {
                child.transport.shutdown().await;
            }
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

async fn respond(hub: Arc<Hub>, request: Value) -> Value {
    let id = request["id"].clone();
    let params = &request["params"];
    match request["method"].as_str().unwrap_or("") {
        "initialize" => {
            let version = normalize::negotiated_version(&params["protocolVersion"]).to_owned();
            json!({"jsonrpc":"2.0","id":id,"result":{
                "protocolVersion":version,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":NAME,"title":"ComandOS","version":env!("CARGO_PKG_VERSION")},
                "instructions":hub.instructions().await,
            }})
        }
        _ => {
            let arguments = params
                .get("arguments")
                .filter(|a| a.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match params["name"].as_str().unwrap_or("") {
                SEARCH => hub.search(&arguments).await,
                CALL => hub.call(&arguments).await,
                other => return error(id, -32602, &format!("Unknown tool: {other}")),
            };
            json!({"jsonrpc":"2.0","id":id,"result":result})
        }
    }
}

pub async fn serve(home: &Path, catalog: &Path) -> Result<()> {
    let hub = Arc::new(Hub::new(home, catalog)?);
    let (input_tx, mut input_rx) = mpsc::channel(8);
    let (end_tx, mut end_rx) = watch::channel(false);
    let input = tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin());
        loop {
            match line(&mut reader).await {
                Ok(Some(bytes)) => {
                    if input_tx.send(bytes).await.is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = end_tx.send(true);
                    break;
                }
            }
        }
    });
    let mut signals = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "Signal setup failed")?;
    let (output_tx, mut output_rx) = mpsc::channel::<Value>(8);
    let (output_end, mut output_closed) = watch::channel(false);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(value) = output_rx.recv().await {
            let mut bytes = value.to_string().into_bytes();
            bytes.push(b'\n');
            if stdout.write_all(&bytes).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
        let _ = output_end.send(true);
    });
    let mut jobs: JoinSet<(String, Value)> = JoinSet::new();
    let mut running: HashMap<String, AbortHandle> = HashMap::new();
    loop {
        tokio::select! {
            biased;
            _=end_rx.changed()=>break,
            _=signals.recv()=>break,
            _=tokio::signal::ctrl_c()=>break,
            _=output_closed.changed()=>break,
            Some(done)=jobs.join_next(),if !jobs.is_empty()=>{
                if let Ok((key,response))=done {
                    running.remove(&key);
                    if output_tx.send(response).await.is_err() {break;}
                }
            },
            bytes=input_rx.recv()=>{
                let Some(bytes)=bytes else {break};
                let request:Value=match comandos_core::json::parse_slice(&bytes) {
                    Ok(v)=>v,
                    Err(_)=>{if output_tx.send(error(Value::Null,-32700,"Parse error")).await.is_err(){break;}continue;}
                };
                let method=request["method"].as_str().unwrap_or("").to_owned();
                if request.get("id").is_none() {
                    if method=="notifications/cancelled" {
                        let key=request["params"]["requestId"].to_string();
                        if let Some(handle)=running.remove(&key) {handle.abort();}
                    }
                    continue;
                }
                let id=request["id"].clone();
                let key=id.to_string();
                let immediate=if request["jsonrpc"]!="2.0" || !matches!(id,Value::String(_)|Value::Number(_)) || method.is_empty() {
                    Some(error(id,-32600,"Invalid request"))
                } else {
                    match method.as_str() {
                        "ping"=>Some(json!({"jsonrpc":"2.0","id":id,"result":{}})),
                        "tools/list"=>Some(json!({"jsonrpc":"2.0","id":id,"result":{"tools":tool_definitions()}})),
                        "resources/list"=>Some(json!({"jsonrpc":"2.0","id":id,"result":{"resources":[]}})),
                        "resources/templates/list"=>Some(json!({"jsonrpc":"2.0","id":id,"result":{"resourceTemplates":[]}})),
                        "prompts/list"=>Some(json!({"jsonrpc":"2.0","id":id,"result":{"prompts":[]}})),
                        "initialize"|"tools/call" if running.contains_key(&key) || running.len()>=MAX_INFLIGHT => Some(error(id,-32000,"Request limit reached")),
                        "initialize"|"tools/call"=>None,
                        _=>Some(error(id,-32601,"Method not found")),
                    }
                };
                if let Some(response)=immediate {
                    if output_tx.send(response).await.is_err() {break;}
                    continue;
                }
                let hub=hub.clone();
                let task_key=key.clone();
                let handle=jobs.spawn(async move {(task_key,respond(hub,request).await)});
                running.insert(key,handle);
            },
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    input.abort();
    writer.abort();
    hub.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_and_fold_keep_text_short_and_accent_free() {
        assert_eq!(clip("  uno\n dos   tres ", 50), "uno dos tres");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(fold("Envíar Correo ÑU"), "enviar correo nu");
        assert_eq!(
            words("gmail-send_message: Envía"),
            ["gmail", "send", "message", "envia"]
        );
    }

    #[test]
    fn summary_prefers_title_then_first_instruction_sentence() {
        assert_eq!(
            summary_of(&json!({"serverInfo":{"title":"Correo"}})),
            "Correo"
        );
        assert_eq!(
            summary_of(
                &json!({"serverInfo":{},"instructions":"\n# Gmail de Jesús. Lee y envía.\nMás"})
            ),
            "Gmail de Jesús"
        );
        assert_eq!(summary_of(&json!({})), "");
    }

    #[test]
    fn tool_definitions_are_two_spanish_tools() {
        let tools = tool_definitions();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [SEARCH, CALL]);
        assert!(tools.to_string().len() < 3000);
    }
}
