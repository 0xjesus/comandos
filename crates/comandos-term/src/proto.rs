//! Protocolo de la terminal sobre WebSocket con dos dialectos.
//!
//! - `tty`: el de ttyd 1.6.3, byte a byte (lo hablan `dash/term.html`, la UI
//!   embebida de ttyd y el teléfono). El primer mensaje del cliente es JSON
//!   crudo sin prefijo `{"AuthToken":"","columns":C,"rows":R}`; después
//!   `'0'`+bytes (entrada), `'1'`+JSON (resize), `'2'` (pausa) y `'3'`
//!   (reanudar). El servidor manda `'0'`+bytes (salida), `'1'`+texto (título)
//!   y `'2'`+JSON (preferencias). Token y sesión llegan como `arg` repetidos en
//!   la consulta (`--url-arg`).
//! - `comandos.term.v1`: mismo juego de mensajes; solo cambia el init
//!   (`{"v":1,"cols":C,"rows":R,"session":"…"}`), porque la puerta HTTP ya
//!   autenticó y la sesión viaja en el propio init.
//!
//! El par del WebSocket no es de confianza: toda trama se acota antes de
//! decodificarla y ningún tamaño sale del rango que acepta el motor.
//!
//! Una trama de cliente que pase de [`MAX_CLIENT_FRAME`] da
//! [`ProtoError::TooLarge`]; el puente de A3 la descarta con un aviso en el
//! registro y **no** cierra el socket (una sesión viva no se corta por un
//! pegado desmedido).

use serde_json::Value;

/// Subprotocolos que acepta el servidor, en orden de preferencia.
pub const PROTOCOLS: &[&str] = &["comandos.term.v1", "tty"];

/// Carga máxima de una trama de entrada (`'0'`+bytes), sin el prefijo: 1 MiB.
///
/// ttyd 1.6.3 acepta cualquier tamaño y ni `dash/term.html` ni la UI de ttyd
/// trocean un pegado, así que un pegado grande llega en una sola trama y no
/// puede fallar. Una trama mayor la descarta el puente (A3) con un aviso en
/// el registro, sin cerrar el socket (ver la documentación del módulo).
pub const MAX_INPUT: usize = 1024 * 1024;

/// Trama de cliente más larga que se acepta: el prefijo más `MAX_INPUT`.
pub const MAX_CLIENT_FRAME: usize = MAX_INPUT + 1;

/// Límite de las tramas JSON de control (init y resize). Los clientes reales
/// mandan menos de 100 bytes; 4 KiB deja margen al `AuthToken` de la UI de
/// ttyd sin dejar que un par hostil haga trabajar al parser.
const MAX_CONTROL_JSON: usize = 4 * 1024;

/// Rango de columnas aceptado.
const COLS: std::ops::RangeInclusive<u16> = 2..=1000;
/// Rango de filas aceptado.
const ROWS: std::ops::RangeInclusive<u16> = 1..=500;

/// Preferencias que ttyd 1.6.3 manda en la trama `'2'` con las opciones `-t`
/// de `bin/cc-webterm`.
///
/// ttyd convierte cada `-t clave=valor` con `json_tokener_parse` (si el valor
/// no es JSON queda como cadena), las añade en orden de inserción y las
/// serializa con `json_object_to_json_string`, que usa el formato «espaciado»
/// de json-c (`{ "k": v, … }`). Este literal se obtuvo de la libjson-c 0.15
/// que enlaza `/usr/bin/ttyd` siguiendo ese mismo algoritmo.
///
/// La prueba diferencial del servidor contra ttyd 1.6.3 vigila este literal
/// byte por byte (incluidos orden y espacios de json-c).
pub const TTY_PREFS: &str = concat!(
    r##"{ "theme": { "background": "#0A0D13", "foreground": "#EAF0FB", "cursor": "#FFAE1A", "##,
    r##""selectionBackground": "#2E3852" }, "fontSize": 11, "##,
    r##""fontFamily": "Ubuntu Sans Mono, JetBrainsMono Nerd Font Mono, JetBrainsMono Nerd Font, "##,
    r##"JetBrains Mono, DejaVu Sans Mono, monospace", "rendererType": "canvas", "##,
    r##""scrollback": 10000, "cursorBlink": true, "disableLeaveAlert": true, "##,
    r##""disableResizeOverlay": true }"##,
);

/// Dialecto negociado por el subprotocolo del WebSocket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Tty,
    V1,
}

impl Dialect {
    /// `"tty"` → `Tty`, `"comandos.term.v1"` → `V1`; cualquier otro, `None`.
    pub fn from_protocol(p: Option<&str>) -> Option<Dialect> {
        match p {
            Some("tty") => Some(Dialect::Tty),
            Some("comandos.term.v1") => Some(Dialect::V1),
            _ => None,
        }
    }
}

/// Errores al decodificar una trama del cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoError {
    BadJson,
    BadSize,
    BadVersion,
    Unknown,
    TooLarge,
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProtoError::BadJson => "JSON de control inválido",
            ProtoError::BadSize => "tamaño de terminal fuera de rango",
            ProtoError::BadVersion => "versión de protocolo no soportada",
            ProtoError::Unknown => "mensaje de terminal desconocido",
            ProtoError::TooLarge => "trama de terminal demasiado grande",
        })
    }
}

impl std::error::Error for ProtoError {}

/// Mensaje del cliente después del init (igual en ambos dialectos).
#[derive(Debug, PartialEq, Eq)]
pub enum ClientMsg {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Pause,
    Resume,
}

/// Primer mensaje del cliente: tamaño con el que nace el PTY y, en v1, la
/// sesión pedida.
#[derive(Debug, PartialEq, Eq)]
pub struct Init {
    pub cols: u16,
    pub rows: u16,
    pub session: Option<String>,
    /// v1 `"ignoreSize":true`: this client must never size the shared tmux window
    /// (a phone showing one pane of a desktop split).
    pub ignore_size: bool,
}

/// Lee `cols_key`/`rows_key` como enteros dentro de los rangos aceptados.
fn size(v: &Value, cols_key: &str, rows_key: &str) -> Result<(u16, u16), ProtoError> {
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
    };
    match (get(cols_key), get(rows_key)) {
        (Some(c), Some(r)) if COLS.contains(&c) && ROWS.contains(&r) => Ok((c, r)),
        _ => Err(ProtoError::BadSize),
    }
}

/// JSON de control acotado: corta por tamaño antes de llamar al parser.
fn control_json(body: &[u8]) -> Result<Value, ProtoError> {
    if body.len() > MAX_CONTROL_JSON {
        return Err(ProtoError::TooLarge);
    }
    serde_json::from_slice(body).map_err(|_| ProtoError::BadJson)
}

/// Decodifica el primer mensaje del cliente según el dialecto.
pub fn parse_init(dialect: Dialect, frame: &[u8]) -> Result<Init, ProtoError> {
    let v = control_json(frame)?;
    match dialect {
        Dialect::Tty => {
            // El `AuthToken` se ignora: ttyd corre sin `-c` y el token de
            // ComandOS viaja como primer `arg` de la consulta.
            let (cols, rows) = size(&v, "columns", "rows")?;
            Ok(Init {
                cols,
                rows,
                session: None,
                ignore_size: false,
            })
        }
        Dialect::V1 => {
            if v.get("v").and_then(Value::as_u64) != Some(1) {
                return Err(ProtoError::BadVersion);
            }
            let (cols, rows) = size(&v, "cols", "rows")?;
            Ok(Init {
                cols,
                rows,
                session: v.get("session").and_then(Value::as_str).map(str::to_string),
                ignore_size: v.get("ignoreSize").and_then(Value::as_bool) == Some(true),
            })
        }
    }
}

/// Decodifica una trama posterior al init (texto o binario, ambos dialectos).
pub fn parse_client(frame: &[u8]) -> Result<ClientMsg, ProtoError> {
    if frame.len() > MAX_CLIENT_FRAME {
        return Err(ProtoError::TooLarge);
    }
    let (cmd, rest) = frame.split_first().ok_or(ProtoError::Unknown)?;
    match cmd {
        b'0' => Ok(ClientMsg::Input(rest.to_vec())),
        b'1' => {
            let v = control_json(rest)?;
            let (cols, rows) = size(&v, "columns", "rows")?;
            Ok(ClientMsg::Resize { cols, rows })
        }
        b'2' => Ok(ClientMsg::Pause),
        b'3' => Ok(ClientMsg::Resume),
        _ => Err(ProtoError::Unknown),
    }
}

/// Valores de los `arg` de la consulta, decodificados y en orden, como los
/// entrega ttyd con `--url-arg`: solo cuentan los campos `arg=…` (ttyd compara
/// con el prefijo `"arg="`, así que un `arg` suelto no cuenta), `%XX` y `+`
/// se decodifican.
pub fn tty_args(query: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|field| field.contains('='))
        .flat_map(|field| comandos_core::dashboard_access::query_pairs(field, true))
        .filter(|(key, _)| key == "arg")
        .map(|(_, value)| value)
        .collect()
}

/// Trama con el byte de comando delante.
fn prefixed(prefix: u8, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len().saturating_add(1));
    v.push(prefix);
    v.extend_from_slice(body);
    v
}

/// Salida del PTY: `'0'`+bytes.
pub fn output(bytes: &[u8]) -> Vec<u8> {
    prefixed(b'0', bytes)
}

/// Título de la ventana: `'1'`+texto.
pub fn title(text: &str) -> Vec<u8> {
    prefixed(b'1', text.as_bytes())
}

/// Preferencias del cliente: `'2'`+JSON.
pub fn prefs(json: &str) -> Vec<u8> {
    prefixed(b'2', json.as_bytes())
}

/// Título con el formato de ttyd: `"<command> (<host>)"`
/// (`sprintf("%c%s (%s)", …)` en ttyd 1.6.3).
pub fn tty_title(command: &str, host: &str) -> String {
    format!("{command} ({host})")
}
