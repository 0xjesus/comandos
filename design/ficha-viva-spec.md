# Barra de sesiones K5 + vista previa + feed de Actividad (ficha viva)

Fecha: 2026-11-09. Origen: grill de diseño de 19 rondas (3–9 nov 2026), bitácora en
`design/references.md` (secciones «Ficha viva…» a «Ronda 19»). Maqueta aprobada:
https://claude.ai/artifact/U2gGcJjQH85Xtq4PCg4afz (v23). Veredictos: F5 (barra de arriba
real + feed de actividad en el panel izquierdo), G4 (pregunta y respuesta), K5 (un solo
violeta, más intenso = más urgente), B4 (loseta de hoy + tarjeta al pasar el cursor).

## Reglas que no se negocian

- La barra de sesiones de ARRIBA se queda donde está: mismas columnas, mismo orden fijo,
  misma altura de loseta (27–28 px). Ninguna loseta se mueve ni cambia de tamaño por estado.
  La terminal nunca pierde altura.
- Estados solo de hechos literales: menú numerado en pantalla = te espera (permiso o
  pregunta); texto de límite de uso en pantalla = te espera (falla); «esc to interrupt» /
  «Working (» en pantalla o estado `working` del hook = trabajando. «Sin leer» lo decide la
  app: hubo respuesta nueva del agente después de la última vez que abriste esa sesión.
- Sin semáforo. Un solo color de acento (`--brand`, #8B7CFF en el tema por defecto), con un
  significado: «ComandOS te llama». Más intenso = más urgente:
  - Te espera: loseta con fondo violeta sólido (`--brand`) y texto oscuro (#0A0D13; el
    modelo en rgba(10,13,19,.62)); en el feed, verbo en violeta y barra izquierda violeta.
  - Sin leer: punto violeta de 7 px al final de la loseta; nombre en blanco y negrita 600.
  - Trabajando: aro violeta tenue (60 % de opacidad) de 9 px con un hueco arriba, girando
    (1.1 s por vuelta; quieto con `prefers-reduced-motion` / animaciones desactivadas).
  - Quieta: sin marca; nombre en texto atenuado (78 % texto + 22 % dim), peso 500.
  - Sesión abierta (activa): borde blanco tenue (texto al 55 %), nunca violeta.
  - Sin tono por proyecto, sin rojo/verde/ámbar/azul en losetas ni feed.
- Sin botones en las tarjetas del feed. Responder un permiso: teclas 1/2/3 sobre la
  vista previa, o abrir la sesión.
- Remoto y escritorio iguales (skill `comandos-remoto-y-escritorio`). Celular operable.
- Nunca tocar el tmux del usuario salvo `capture-pane -p` de lectura y el envío de teclas
  ya existente (`/key`). Nada de formatos de tiempo de tmux con `-t`.

## 1. Servidor: `GET /session-feed` y `POST /session-seen`

Rutas nuevas nativas en `comandos-server` (sin paridad con el Python heredado; no tocan
`/state` ni `/states`).

`GET /session-feed` → JSON:

```json
{
  "now": 1794250700.0,
  "sessions": [
    {
      "session": "term-r26549",
      "label": "FerLongoria",
      "state": "ask",
      "unread": false,
      "ts": 1794250552.0,
      "seen": 1794250000.0,
      "pane": "%51",
      "panes": 1,
      "agent": "claude",
      "model": "opus-5",
      "prompt": "pushea todo",
      "reply": "Tengo dos commits locales: 6db667f1 y 5d6b4c7e. ¿Los subo?",
      "ask": {
        "question": "Bash(git push origin main) · Do you want to proceed?",
        "options": [
          {"key": "1", "label": "Yes"},
          {"key": "2", "label": "Yes, and don't ask again for git push commands"},
          {"key": "3", "label": "No, and tell Claude what to do differently (esc)"}
        ]
      }
    }
  ]
}
```

- Una entrada por sesión tmux viva con al menos un pane de agente (claude, codex, grok,
  gemini, agy, opencode…), incluida `local`; fuera `hub` y `control`.
- `pane`: el pane más relevante de la sesión: el que pide (ask) > límite > trabajando >
  el de `ts` más reciente. `panes`: cuántos panes con agente tiene la sesión.
- `state`: `ask` | `limit` | `work` | `idle` (de la sesión: el más urgente de sus panes).
  - `ask`: en las últimas 14 líneas de `capture-pane -p -t <pane> -S -14` hay un menú
    numerado: regex existentes `QUESTION` y `CONFIRM` de `states/cards.rs`, o una línea
    `❯ 1.` seguida de líneas `2.`/`3.`.
  - `limit`: `(?i)usage limit reached|limit reached.*reset|you've hit your (usage )?limit`
    en esas líneas.
  - `work`: estado del hook `working`, o `INTERRUPT`/`WORKING` en pantalla.
- `ask.options`: las líneas `^\s*(?:❯\s*)?([1-9])\.\s+(.+)$` del menú visible, en orden.
  `ask.question`: hasta 300 caracteres de las líneas no vacías justo encima del menú,
  unidas con « · », sin marcos de caja (`│ ╭ ╰ ─`). Si no hay menú, `ask` es `null`.
- `ts`: `ts` del registro del hook (última respuesta / aviso del agente) del pane elegido.
- `prompt`: lo último que el usuario le escribió al agente, leído del final del transcript
  exacto del pane (Claude: `projects/*/<agentSessionId>.jsonl` del config-dir de su cuenta,
  último `type:"user"` con texto propio, sin `tool_result`, sin `isMeta`, sin
  `<command-…>`/`<local-command-…>`/`<system-reminder>`; Codex: último `user_message` del
  rollout). Espacios colapsados, ≤ 400 caracteres. `""` si no se puede saber con certeza.
- `reply`: `detail` del registro si existe, si no `last` (texto real del agente que ya
  escribe el hook), sin markdown pesado (`**`, `` ` ``, `#` al inicio, tablas → texto),
  espacios colapsados, ≤ 1200 caracteres.
- `model`: corto como la pestaña de hoy (`claude-opus-5-5` → `opus-5`, `gpt-6-astra`).
- `seen` / `unread`: `seen` = marca guardada para la sesión (0 si nunca);
  `unread = state != "work" && ts > seen && reply != ""`. `ask` y `limit` cuentan como
  no leídos también.
- Coste: la captura de pantallas se cachea ≥ 1.5 s (varias peticiones seguidas reutilizan
  el resultado); un transcript se relee solo si cambió su tamaño o `mtime`.

`POST /session-seen` con `{"session": "term-r26549"}` → guarda `seen[session] = now` en
`~/.claude/hooks/session-seen.json` (escritura atómica, candado como los demás JSON de
estado) y responde `{"ok": true}`. Misma puerta de seguridad que las demás rutas POST.

## 2. Loseta de la barra de arriba (escritorio GTK y remoto web)

Contenido igual que hoy: `nombre · modelo +N`. Se añade al final la marca K5 (punto o aro)
y las clases de estado (ver reglas). Fuente: `/session-feed` (estado, unread).

Vista previa (B4), solo puntero fino:
- Aparece tras 350 ms con el cursor sobre una loseta; se va al salir (150 ms de gracia).
  No roba foco, no mueve nada, no tapa la loseta: se abre debajo de ella, alineada a su
  borde izquierdo (o al derecho si no cabe), ancho 320 px.
- Contenido (G4): cabecera «Nombre» + verbo (`respondió`, `pide permiso`, `está trabajando`,
  `llegó al límite de uso`, `sin actividad`) + hace cuánto a la derecha; línea «Tú: <prompt>»
  atenuada y en una línea; bloque con barra izquierda (violeta si te espera) con
  `ask.question` y las opciones numeradas si hay menú, si no `reply` (hasta 4 líneas).
  Pie con teclas: «clic abrir» y, si hay menú, «1 2 3 responder».
- Con la vista previa visible sobre una loseta en `ask`, las teclas 1–9 que existan en
  `ask.options` envían ese dígito al pane (`POST /key` con su sesión y pane; misma ruta
  que usan las notificaciones) y cierran la vista previa. Ninguna otra tecla se captura.
- Celular / táctil: mantener presionada la loseta 450 ms abre la misma tarjeta (en el
  remoto, fija bajo la barra, ancho completo menos 16 px), un toque fuera la cierra. En
  la tarjeta del celular las opciones del menú son filas tocables de 44 px (el celular no
  tiene teclas 1 2 3); el toque normal en la loseta sigue abriendo la sesión.

Hover enlazado: pasar el cursor por una fila del feed resalta su loseta (borde blanco
tenue 35 % y fondo +7 %) y al revés.

## 3. Feed «Sesiones» en el panel izquierdo (web; mismo código en escritorio y remoto)

Nueva pestaña en la barra lateral: «Archivos · Consumos · Sesiones», con un contador
violeta sólido = sesiones que te esperan (oculto si 0).

- Arriba, filtros en chips: «Todas N · Sin leer N · Permisos N · Trabajando N».
- «TE ESPERA · N» fijo arriba, fuera del scroll, con cabecera en violeta: sesiones en
  `ask`/`limit`, la más antigua primero.
- Debajo, con scroll, el resto agrupado por hora del `ts`: «AHORA» (< 5 min), «ÚLTIMA
  HORA», «HOY», «ANTES»; dentro, más reciente primero.
- Fila G4 (sin botones): avatar 28 px (iniciales del nombre en gris; glifo ✻ Claude /
  ◎ Codex pequeño en la esquina); línea 1: **Nombre** + verbo + hace cuánto a la derecha
  con la marca K5 (punto si no leído, aro si trabaja); línea 2: «Tú: <prompt>» en una
  línea, atenuada; bloque con barra izquierda (2 px, `--line2`; violeta en te espera) con
  la pregunta del menú o la respuesta, 3 líneas máximo. Sin leer: nombre y texto en blanco;
  leído: atenuados.
- Clic en una fila abre esa sesión y enfoca su pane. Filas cómodas (≥ 56 px), con aire,
  scroll permitido; en celular 44 px mínimos tocables.
- Se refresca con el mismo ritmo que el resto del tablero (sondeo de `/session-feed`
  cada 2–3 s, solo con la página visible).

## 4. Marcar como visto

La app (escritorio: al cambiar de pestaña y en cada sondeo mientras la ventana tiene el
foco y la pestaña está abierta; remoto: al abrir la vista de una sesión y en cada sondeo
mientras está visible) hace `POST /session-seen` de la sesión abierta.

## Fuera de esta entrega

- La ficha de la derecha («esta sesión») no tuvo ronda propia de detalle; queda para la
  siguiente ronda del grill. El pie de la vista previa no la menciona.
- El sonido del hook para `idle_prompt` (Claude esperando sin pregunta) sigue como hoy; la
  regla «solo suenan permisos, preguntas y fallas» queda pendiente porque toca el hook con
  paridad (`hooks/claude.rs`).
