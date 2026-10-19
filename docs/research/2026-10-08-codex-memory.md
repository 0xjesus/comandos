# Consumo de Codex y conectores MCP: medición y causas

Investigación local del 8 de octubre de 2026, aproximadamente 01:30–01:45, America/Mexico_City. Se midió memoria, se revisó el código de ComandOS y de Codex 0.161.0, y se reprodujeron dos defectos con servidores ficticios. No se cambiaron configuraciones operativas ni se detuvieron sesiones para obtener estas conclusiones.

**El consumo observado es real, pero no es un coste inevitable de mantener estas conversaciones.** La mayor parte está en conectores externos repetidos. Hay un defecto reproducido que impide utilizar el servicio de procesos compartidos y otro que debe corregirse antes de extender esa compartición entre cuentas. La migración de la interfaz de ComandOS a Rust no convierte los servidores externos Node/Python a Rust ni elimina sus instancias existentes.

## Qué ocupa la memoria

Dos muestras separadas por 62 segundos. La segunda, 07:31:40 UTC, contiene 2,292 procesos en los árboles de Codex. Cada PID se cuenta una sola vez, incluso con procesos Codex anidados. Se usa PSS de Linux, que reparte las páginas compartidas: sumar RSS en esta máquina daría aproximadamente 75 GiB para esos mismos árboles y exageraría su coste físico conjunto. [Definición del kernel](https://www.kernel.org/doc/html/latest/filesystems/proc.html).

| Componente dentro de los árboles de Codex | PSS, GiB |
|---|---:|
| Codex: 24 procesos, incluidos clientes y app-servers | 10.070 |
| MCP stdio que el catálogo permite compartir: 706 instancias, 1,163 procesos | 24.142 |
| MCP stdio configurados como dedicados | 9.658 |
| Intermediarios de ComandOS hacia MCP HTTP: 668 procesos | 3.968 |
| Aplicaciones de desarrollo, .NET y Vite | 1.533 |
| Runtimes auxiliares de Codex | 0.521 |
| Runtime de plugin CUA | 0.421 |
| Otros procesos pendientes de clasificación fina | 0.040 |
| **Total** | **50.353** |

Los descendientes, excluidos los ejecutables Codex, suman **40.283 GiB**. Esto confirma el orden de magnitud de la medición anterior de 10 GiB + 40.3 GiB. Los 721 intermediarios contabilizados en toda la máquina suman aproximadamente 4.13 GiB; sólo 668 están dentro de los árboles Codex. No son ellos solos quienes ocupan los 40 GiB. Los proxies stdio pueden ejecutar directamente Node/Python y desaparecer como intermediario reconocible.

La medición previa de aproximadamente 0.45 GiB para interfaz, WebKit y servidor ComandOS tenía un alcance diferente. No incluía esta carga de Codex y sus herramientas. El ahorro de compilaciones en disco tampoco implica una reducción equivalente en RAM.

| Servicio, incluyendo sus procesos auxiliares dentro de Codex | PSS, GiB |
|---|---:|
| Teams, dedicado | 5.049 |
| Xano | 4.199 |
| Azure DevOps | 3.936 |
| MCP Image | 3.527 |
| Supabase Develop | 3.089 |
| Supabase | 2.992 |
| Odoo | 2.380 |
| X Suite, dedicado | 2.021 |
| Claude in Chrome, dedicado | 1.852 |

Varios servicios tienen entre 39 y 44 instancias, aunque sólo hay 24 ejecutables Codex persistentes. Los contextos y subagentes también mantienen conexiones; contar ventanas no basta para estimar su coste. La mediana de PSS por ejecutable Codex es 228.8 MiB y el máximo 2,190.3 MiB. No se atribuyó la memoria de ese proceso mayor a una fuga sin perfilarlo.

## Causa reproducida: el broker existe, pero muchos clientes no lo encuentran

El broker de producción escucha en `/run/user/1000/comandos/broker.sock`. El cliente de ComandOS decide la ruta usando `XDG_RUNTIME_DIR`; si la variable falta, busca `/tmp/comandos-1000/broker.sock`. Al no encontrarlo, arranca el servidor directamente. Implementación en `/home/someguy/codebase/0xJesus/ComandOS/crates/comandos-extensions/src/broker/mod.rs:119` y `/home/someguy/codebase/0xJesus/ComandOS/crates/comandos-extensions/src/cli.rs:129`.

La fuente oficial de Codex 0.161.0 explica cómo puede perderse esa variable: limpia el entorno del proceso MCP y reconstruye una lista predeterminada que no incluye `XDG_RUNTIME_DIR`. Variables adicionales deben reenviarse expresamente. [Lista predeterminada](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/rmcp-client/src/utils.rs#L162-L175), [limpieza del entorno](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/utils/pty/src/child_command.rs#L94-L103).

La inspección local encontró esa variable ausente en los procesos afectados. Sus fechas de inicio son posteriores al broker: el problema también alcanza conexiones recientes y no se explica sólo por sesiones antiguas.

Prueba aislada con el binario instalado, mismo servidor ficticio y tres clientes:

| Condición | Procesos reales del servidor | initialize / tools/list |
|---|---:|---|
| Variable presente y apuntando al broker privado | 1 | Correctos |
| Variable ausente | 3 | Correctos, mediante ejecución directa |

Los seis clientes salieron al cerrar su entrada. Se cerraron únicamente los procesos creados por la prueba, sin llamadas a servicios externos.

## Segundo defecto: aislar la cuenta antes de compartir más

La identidad actual del broker incluye nombre, directorio y especificación con el entorno declarado. Sin embargo, el proceso recibe también el entorno heredado del primer cliente, que no participa completo en esa identidad. Código en `/home/someguy/codebase/0xJesus/ComandOS/crates/comandos-extensions/src/broker/mod.rs:31` y `/home/someguy/codebase/0xJesus/ComandOS/crates/comandos-extensions/src/broker/registry.rs:77`.

Una segunda prueba privada usó dos valores ficticios de `ACCOUNT_ID`, sin credenciales reales. Ambos clientes compartieron un único servidor y el segundo recibió el identificador del primero. Esto demuestra un defecto de aislamiento por entorno; no demuestra que ya se haya producido un cruce de cuentas reales.

Por eso reenviar la variable que falta no es todavía la corrección completa. El descubrimiento del socket y la separación de identidad efectiva deben resolverse juntos. Deben probarse también el estado por cliente, las capacidades, las notificaciones y las llamadas simultáneas. El transporte MCP permite servidores HTTP con múltiples clientes, pero no elimina la obligación de mantener sus sesiones separadas. [Especificación MCP](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports).

## Ahorro estimado y límites

Dentro de los 17 servicios stdio que el catálogo considera compartibles se identificaron 706 instancias. Agrupando por ejecutable, argumentos, directorio, catálogo, configuración y **todo el entorno heredado**, quedan 352 identidades distintas.

Para evitar una estimación optimista se conserva la copia de mayor PSS de cada grupo y se calcula la diferencia con la suma actual. Resultado: **12.261 GiB de memoria de procesos duplicados candidata a ahorro**. Usar sólo el entorno declarado daría 14.048 GiB, pero ese escenario no mantiene toda la separación observada y no es la recomendación.

Los 12.261 GiB no son memoria ya liberada ni una garantía. Falta descontar el coste de los clientes del broker y medir cuánto crece un servidor al atender varias conexiones. También falta validar qué estado necesita aislamiento adicional. Los servidores dedicados y los intermediarios HTTP quedan fuera de esta estimación.

El orden recomendado para una implementación posterior es:

1. Corregir descubrimiento seguro del socket y la identidad del proceso compartido. Probar misma cuenta, cuentas distintas, cwd diferente, concurrencia, desconexión y recuperación.
2. Usar el camino corregido primero en conexiones nuevas. Las instancias ya abiertas no desaparecen al sustituir el ejecutable. No reconectar herramientas con trabajos en vuelo ni reiniciar todas las sesiones para obtener una cifra menor.
3. Preparar perfiles por tarea con los MCP necesarios. Teams, X Suite y Claude in Chrome suman unos 8.92 GiB en esta muestra; mantenerlos dedicados es distinto de cargarlos en todas las tareas. Desactivarlos donde no se necesitan puede aportar ahorro adicional, aún no cuantificado por perfil.
4. Medir por separado la compartición de los proxies HTTP. Sus aproximadamente 4 GiB son otro objetivo, pero no forman parte del ahorro de 12.261 GiB.
5. Revisar concurrencia y vida de subagentes, y después perfilar los procesos Codex mayores. Priorizar la duplicación comprobada antes que ajustar a ciegas límites de memoria del runtime.

Codex permite desactivar servidores mediante `enabled = false`; filtrar herramientas no equivale a evitar que arranque su servidor. Los perfiles actuales son archivos independientes, y la concurrencia pública se controla mediante `agents.max_concurrent_threads_per_session`, con posibles anulaciones avanzadas de V2. Estas opciones deben aplicarse a tareas apropiadas y medirse, no imponerse globalmente a sesiones activas. [Configuración MCP](https://learn.chatgpt.com/docs/extend/mcp), [perfiles](https://learn.chatgpt.com/docs/config-file/config-advanced#profiles), [subagentes](https://learn.chatgpt.com/docs/agent-configuration/subagents#global-settings).

No existe `/mcp refresh` en la TUI revisada. La API `config/mcpServer/reload` recarga MCP para los hilos cargados del app-server y no acepta un hilo individual. Su existencia no prueba una transición sin interrupciones de llamadas activas. [Comandos TUI](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L839-L857), [protocolo de recarga](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/app-server-protocol/src/protocol/common.rs#L1233-L1236).

## Qué es esperable en Codex y qué no quedó demostrado

Codex comparte catálogos entre algunos hilos, pero cada hilo tiene su runtime MCP. Los primarios usan arranque inmediato; los subagentes pueden diferir conexiones cuando existe un catálogo compatible. Por tanto, ni todas las conexiones se comparten ni todos los subagentes arrancan siempre todo. [Gestor compartido](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/core/src/thread_manager.rs#L560-L564), [política de arranque](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/core/src/session/mcp_runtime.rs#L379-L395).

El caché de catálogos tiene un índice de 32 entradas y vigencia de 30 minutos; la configuración local contiene 39 MCP. Esto justifica investigar la eficacia del caché, pero no prueba saturación, fugas ni un ahorro concreto. Las identidades y accesos reales importan, y otros consumidores pueden conservar entradas desalojadas. [Implementación](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/codex-mcp/src/tool_catalog_cache.rs#L33-L105).

La compacción resume contexto del modelo; no fija un límite de RAM y no sustituye el apagado de MCP. Codex conserva otros objetos y snapshots, y estos binarios Linux musl usan jemalloc. No se midió qué proporción corresponde a objetos vivos o a páginas retenidas por el asignador. [Compacción](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/core/src/tasks/compact.rs#L28-L81), [historial y snapshots](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/core/src/context_manager/history.rs#L90-L110), [asignador](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/cli/src/main.rs#L45-L51).

El total bajó 89.1 MiB entre muestras, principalmente al terminar un helper transitorio. Para los mismos procesos, el cambio fue aproximadamente +4 MiB; en los Codex persistentes, +12.1 MiB. Una ventana de 62 segundos no demuestra ni descarta una fuga lenta. No hay base aquí para afirmar que todo el consumo sea una fuga de Codex.

En una medición adicional de cinco segundos, la máquina tenía 93.96 GiB de RAM, 13.66 GiB disponibles y prácticamente llenos sus 18 GiB de swap. No hubo páginas entrando o saliendo de swap ni fallos mayores; las medias PSI de presión de memoria registraron 0.00. Swap ocupada no equivale a intercambio activo en ese momento; esta muestra no atribuye toda la lentitud del terminal a RAM.

También se encontraron cinco intermediarios huérfanos retenidos después de perder su escritor de entrada y siete brokers de pruebas antiguos sin upstreams. Son candidatos a corregir en el ciclo de vida de los procesos, pero no explican los aproximadamente 40 GiB de descendientes. La investigación los dejó intactos.

## Evidencia y preservación

Código oficial fijado al commit `979011409de0a60b52f179721948e65531d26144`, tag `rust-v0.161.0`. Se verificó CLI y daemon instalados 0.161.0 y el binario incluido con desktop 0.160.1. La comparación de ejecutables vivos encontró 21 CLI y dos daemons con el mismo hash que el binario 0.161.0; queda un app-server de desktop distinto. No se atribuyen automáticamente todas las estructuras de 0.161.0 a ese proceso.

Artefactos locales de auditoría, sin valores de argumentos ni entorno en el informe agregado:

- `/home/someguy/codebase/0xJesus/ComandOS/.scratch/codex-memory-research-20261008/enriched-report.json`
- `/home/someguy/codebase/0xJesus/ComandOS/.scratch/codex-memory-research-20261008/mcp-broker-isolated-check.json`
- `/home/someguy/codebase/0xJesus/ComandOS/.scratch/codex-memory-research-20261008/mcp-broker-account-isolation-check.json`
- `/home/someguy/codebase/0xJesus/ComandOS/.scratch/codex-memory-research-20261008/system-pressure.json`
- `/home/someguy/codebase/0xJesus/ComandOS/.scratch/codex-memory-research-20261008/session-preservation.json`

La comprobación final conserva los 24 PID, ejecutables y horas de arranque de Codex del inventario original. Hay 23 de los 24 paneles tmux de aquel inventario; la auditoría no determina cuándo desapareció el restante y no atribuye una causa. No se enviaron señales a procesos operativos. Las pruebas sólo crearon y cerraron sus propios procesos ficticios. Este informe documenta una investigación terminada; la corrección operativa y el ahorro efectivo siguen pendientes.
