# Referencias de controles de sesión

El trabajo mejora controles funcionales de ComandOS y su uso desde un teléfono. La implementación conserva la tipografía, los colores y los componentes existentes.

Mobbin no aparece entre las herramientas conectadas de esta sesión. No se consultaron pantallas de Mobbin ni se atribuyen decisiones a referencias de esa plataforma. No se generaron propuestas visuales alternativas: se implementaron los controles sobre la interfaz existente y se verificaron con pruebas aisladas de navegador.

| Fuente consultada | Qué se revisó | Uso |
|---|---|---|
| [Interfaz existente](../dash/index.html) | Tema, selector, tarjetas, chat y formulario de inicio. | Sí: reutilización de estilos y flujos. |
| [Terminal existente](../dash/term.html) | Gestos, entrada, historial y barra de acciones. | Sí: selección nativa y compositor sobre el terminal existente. |
| [SQLite: usos apropiados](https://www.sqlite.org/whentouse.html) | Almacenamiento local de una aplicación. | Sí: decisión de persistencia; no es una referencia visual. |
| [SQLite: WAL](https://www.sqlite.org/wal.html) | Concurrencia de lectura y escritura. | Sí: persistencia local; no es una referencia visual. |

La dirección aplicada es un selector con un solo envío, tarjetas para comparar sesiones y controles táctiles accesibles. Un mosaico que renderice todas las terminales no se usa para el resumen, para evitar trabajo de renderizado innecesario.

Los archivos del trabajo visual son `DESIGN.md`, `proof.html` y `shots/`. Las capturas muestran datos de prueba; no son evidencia de validación en un teléfono físico.


## Pomodoro: pixel art de artistas, 2026-09-28

Jesús rechazó los vectores arcade dibujados para el mockup y pidió assets pixel art abiertos de mejor calidad. La distribución B, Regla de tiempo, sigue elegida. Esta ronda compara cinco conjuntos dentro del mismo mockup, sin reabrir la distribución ni cambiar producción. Las búsquedas y lecturas de las páginas se hicieron con la skill busqueda-web; después se descargaron los archivos gratuitos publicados por los autores. Las imágenes se inspeccionaron directamente y en el prototipo.

| Fuente | Qué se revisó | Uso y motivo |
|---|---|---|
| [Shikashi, pack gratuito](https://shikashipx.itch.io/shikashis-fantasy-icons-pack) | Sprites transparentes de 32 px, reloj de arena, gemas, fogata, plantas, cofre. Página actual CC BY 4.0; notas del ZIP de 2020 conservan referencia CC BY 3.0 de game-icons.net. | Sí. Conjuntos Fantasía y Jardín; reloj y cofre compartidos con Arcade. Se acreditan Matt Firth y game-icons.net. |
| [La Red Games, Gems / Coins Free](https://laredgames.itch.io/gems-coins-free) | Tiras originales de monedas y gemas de 16 px, licencia CC0 indicada por el autor. | Sí. Propuesta inicial Arcade, con animación real de cinco cuadros para monedas y cuatro para gemas. |
| [Quintino Pixels, Free Assorted Icons](https://quintino-pixels.itch.io/assorted-icons) | PNG transparentes de 32 px: reloj, frascos, llave y moneda. CC0 en título y confirmación del autor. | Sí. Conjunto Alquimia, con movimiento CSS añadido a iconos estáticos. |
| [7Soul, compilación en OpenGameArt](https://opengameart.org/content/496-pixel-art-icons-for-medievalfantasy-rpg) | Reloj, medallas, cristal, cofre y fuego. Compilación CC0 que excluye derivados de juegos comerciales. | Sí. Conjunto RPG clásico. Se conservan atribución y referencias de licencia ante las etiquetas históricas diferentes. |
| [7Soul, publicación del autor](https://www.deviantart.com/7soul1/art/420-Pixel-Art-Icons-for-RPG-129892453) | Descripción del autor: Public Domain y eliminación de derivados; metadatos aún CC BY 3.0. | Sí. Corroboración y atribución conservada a Henrique Lazarini. No se presenta la inconsistencia como resuelta. |
| [7Soul, pack actual de pago](https://7soul.itch.io/7souls-rpg-graphics-pack-1-icons) | Producto distinto, más de 1700 iconos. | No. No se compró ni se copió; no confundir con la compilación antigua abierta. |
| [Kenney, Pixel UI Pack](https://kenney.nl/assets/pixel-ui-pack) | Preview y ZIP CC0 de paneles, botones y flechas. | No. Útil para interfaz, pero carece del conjunto de reloj y recompensas que necesita esta ronda. |

El inventario, hashes de archivos y créditos de los packs están en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/assets/pixel-packs. Los PNG originales no se modificaron. CSS selecciona celdas y añade movimiento; La Red conserva sus cuadros originales. La elección visual sigue pendiente del usuario.


### Alquimia: el usuario exige movimiento por fotogramas

Tras elegir Alquimia, Jesús pidió sprites cuya animación cambie realmente el dibujo. La propuesta anterior de Quintino era estática y se movía con CSS; no satisface ese nuevo requisito. La siguiente versión conserva la dirección de alquimia y usa fotogramas originales de karsiori, con sus recipientes inmóviles mientras se animan burbujas, líquido y destellos.

| Fuente | Revisión | Uso |
|---|---|---|
| [karsiori, pociones animadas](https://karsiori.itch.io/pixel-art-potion-pack-animated) | Licencia CC0 explícita, spritesheets PNG, rate sugerido de 10 fps. Se inspeccionó el archivo original y sus cuadros. | Sí: seis sprites de 7 a 24 fotogramas. Una poción de foco reemplaza visualmente el reloj de arena estático; el contador sigue mostrando el tiempo real del mockup. |
| [karsiori, llaves animadas](https://karsiori.itch.io/pixel-art-key-pack-animated) | Página y licencia CC0; 10 tipos animados. | No: alternativa disponible para futuros logros, no se descargó ni mezcló en esta revisión. |
| [karsiori, cofres animados](https://karsiori.itch.io/pixel-art-chest-pack-animated) | Página y licencia CC0; secuencias de apertura y cierre. | No: esta revisión muestra una familia de pociones coherente. |
| [karsiori, faroles animados](https://karsiori.itch.io/free-pixel-art-lantern-pack) | Página y licencia CC0; siete tipos animados. | No: posible recurso posterior, no se descargó. |

La galería ofrece una vista de los fotogramas originales. Es una herramienta de revisión del mockup, no un control de producción. Se debe validar visualmente esta nueva familia con el usuario; aceptar el estilo Alquimia no implica haber aprobado ya los sprites concretos de karsiori.


### Catálogo de estilos de Pomodoro

Jesús pidió conservar varios estilos intercambiables, incluidos los que ya había visto, en vez de elegir uno para excluir los demás. El catálogo mantiene Alquimia, Arcade, Fantasía, RPG clásico y Jardín. Añade Cristales con sprites originales de karsiori. La elección aplica en un toque y conserva el tiempo, los XP y los logros.

| Fuente | Revisión | Uso |
|---|---|---|
| [karsiori, gemas animadas](https://karsiori.itch.io/free-pixel-art-gem-pack) | Página CC0, seis hojas PNG originales inspeccionadas; 10 u 11 cuadros según la gema. | Sí: nuevo estilo Cristales, con seis símbolos por fotogramas. |
| [karsiori, flores](https://karsiori.itch.io/free-pixel-art-flower-pack) | Licencia CC0; ilustraciones estáticas, no confirma animación. | No: no resuelve el requisito de sprites para Jardín. |
| [karsiori, pinos animados](https://karsiori.itch.io/spruce-tree-pack-pixel-art-animated) | Licencia CC0; árboles animados de 52 a 128 px de alto. | No: requeriría rediseñar Jardín y sus tamaños. Se conservan las plantas que el usuario ya vio. |
| [Perfil de karsiori](https://karsiori.itch.io/) | Catálogo de packs del artista y enlaces a gemas, flores y árboles. | Sí: descubrimiento de las fuentes; no se copiaron otras ilustraciones. |

Las tarjetas distinguen seis sprites originales, combinación de sprites/iconos, o iconos estáticos con movimiento CSS. No se presenta el movimiento CSS como animación dibujada. Convertir Fantasía, RPG clásico y Jardín a sprites originales completos sigue pendiente de diseño. La preferencia global de producto se propone para desktop y remoto; el prototipo guarda solo el estilo en el navegador y no sincroniza equipos.


### Visible action, rather than subtle glints, for Pomodoro

Jesús reports that the clock appears still and asks for more expressive original animations. Inspection reproduced the mismatch: Fantasía, Arcade, RPG clásico and Jardín still used static clock art with outer CSS movement; the separate 20 px header hand did animate, but its idle revolution took 24 seconds. Chrome on the remote Mac confirmed changing transforms and reduced motion disabled. This was an artwork/visibility problem in the mockup; no evidence ties it to tmux.

| Source | Verified evidence | Decision |
|---|---|---|
| [Zoedoz, Animated Hourglass](https://opengameart.org/content/animated-hourglass) | CC0; unchanged 15-frame PNG plus Aseprite source establishing 42×42 geometry and 100 ms timing. Frames show sand falling and the glass turning over. | Use as the common clock in all styles and the header. |
| [karsiori, animated chests](https://karsiori.itch.io/pixel-art-chest-pack-animated) | Explicit CC0; free ZIP contains Golden Chest 1, five distinct original frames. | Use in Arcade, Fantasía and RPG clásico; show opening/holding/closing in the review gallery. |
| [ArlanTR, animated campfire](https://opengameart.org/content/campfire-pixel-art-animated) | CC0; four distinct original 32×32 frames. | Replace the static fire in Fantasía and RPG clásico; include in the motion review. |
| [William.Thompsonj and Sharm, LPC Clock Animation](https://opengameart.org/content/lpc-clock-animation) | CC BY 3.0 / GPL 3.0 options; original sheet inspected. Tall pendulum clock; movement concentrated at the base. | Omit: the hourglass communicates motion more clearly at small UI sizes. |
| [Robert Brooks, animated sand timers](https://gamedeveloperstudio.itch.io/animated-sand-timers) | Paid download, £5 minimum, turn sequence and 15 states. | Omit; not an open/free pack and not purchased. |
| [ollieMarsh, animated cauldron](https://makinggamesinc.itch.io/cauldron-pixel-art) | Paid download, $1 minimum, nine frames. | Omit; not purchased and redistribution terms were not established. |

This revises the earlier note saying no animated hourglass was found. The shared hourglass and other new artwork are proposals for review, not a recorded user acceptance. The six styles remain selectable; unrelated static symbols remain clearly labeled.


### Notification placement round

The reference is the current ComandOS sidebar served by the isolated fixture, alongside the already accepted terminal workspace direction. Five alternatives compare placement of the same notice content: side inbox, floating notice, bottom strip, contextual tab/pane indicators and a full notification center. No external visual references or stock illustrations were used in this round. The line icons are inline SVG; the existing optional UISFX controller supplies local one-shot sound previews.

The approved most-recent-device sound policy is represented in fixture state only. Reliable event identity, real delivery and synchronization still require production work. In particular, the production focus queue must preserve the source pane identity if a later notification is expected to return to that exact pane. Notification suppression during Pomodoro remains an unresolved product choice.

## Referencias para complementar las decisiones del grill

La selección masiva de Carbon, las configuraciones reutilizables de Raycast y la bandeja de Linear aportan patrones concretos para revisar en ComandOS. Son propuestas complementarias a las decisiones aceptadas. Jesús pide investigación de referencias antes de plantear más refactorizaciones.

La consulta del 28 de septiembre de 2026 usa la skill 0xai-design-research. Mobbin es el motor solicitado y está configurado, pero su MCP falla durante initialize: connection closed. No se consultaron pantallas de Mobbin ni se accedió mediante scraping. La investigación disponible procede de documentación oficial, imágenes publicadas por los productos y Chrome remoto en la Mac mini.

El alcance es comparar interacciones para desktop y remoto. Se conservan el drag/drop de tabs, la restauración automática, los estados independientes, la regla de tiempo y la franja inferior con flotantes. Una referencia externa no cambia esas decisiones. Las diferencias de gestos y distribución móvil deben conservar las capacidades acordadas.

| Referencia | Evidencia consultada | Uso propuesto y límite |
|---|---|---|
| [Mobbin MCP](https://api.mobbin.com/mcp) | Intento de iniciar la conexión; fallo de handshake. | No utilizado como evidencia visual. Pendiente de conexión funcional. |
| [Linear Inbox](https://linear.app/docs/inbox) | Documentación, página en Chrome remoto y dos imágenes oficiales: bandeja y opciones de presentación. | Parcial: separar atención pendiente, lectura y posponer dentro de nuestra franja. No copiar su navegación lateral. |
| [Linear Notifications](https://linear.app/docs/notifications) | Documentación de canales, indicadores de habilitación y envío condicionado a lectura para resúmenes de correo. | Parcial: hacer explícito el estado de cada canal. No atribuir su política de correo a Telegram ni adoptarla automáticamente. |
| [Linear Mobile](https://linear.app/mobile) | Página oficial leída: acciones táctiles, posponer y horario de notificaciones. | Parcial: acciones alcanzables en móvil. Su enfoque de cliente complementario no satisface por sí solo la paridad completa requerida en ComandOS. |
| [VS Code Custom Layout](https://code.visualstudio.com/docs/configure/custom-layout) | Documentación, sección Editor groups inspeccionada en Chrome y captura oficial de separadores. | Parcial: ampliar temporalmente un grupo y volver a su tamaño, conservando el diseño elegido. Sus gestos de escritorio requieren adaptación táctil; no se proponen menús direccionales. |
| [VS Code Extension Marketplace](https://code.visualstudio.com/docs/configure/extensions/extension-marketplace) | Documentación de publisher, identificadores, filtros y habilitación por workspace. | Parcial: procedencia verificable y alcance visible. Instalado, habilitado, seleccionado y cargado por un proceso deben seguir siendo conceptos distintos. |
| [Raycast Agents](https://manual.raycast.com/ai/agents) | Documentación y dos capturas oficiales: configuración y agente activo en chat. | Parcial: configuraciones con nombre que reúnen modelo, instrucciones y herramientas; selección visible junto al compositor. La fuente no establece recomendaciones deterministas de costo/beneficio. |
| [TickTick Start Focus](https://help.ticktick.com/articles/7055782010496745472) | Documentación leída y página inspeccionada en Chrome; describe móvil, desktop, cambios de tiempo y sincronización. | Parcial: comenzar desde el contexto de trabajo y ajustar tiempo directamente. Se conserva nuestra regla horizontal. No se validó una sesión real del producto. |
| [TickTick Focus Statistics](https://help.ticktick.com/articles/7055781966800486400) | Documentación de tendencias, periodos, distribución, historial y filtros. | Parcial: relacionar totales con registros visibles. Sus estadísticas no prueban atención humana ni definen nuestras reglas de XP. |
| [Carbon Data Table](https://carbondesignsystem.com/components/data-table/usage/) | Documentación y captura del ejemplo de acciones por lote desde Chrome remoto. | Parcial: checkbox con estado parcial, contador de selección y barra de acciones; controles disponibles en touch y reserva de espacio al cargar. No convertir por eso toda la interfaz en tablas. |

La búsqueda adicional de selección masiva en Gmail devolvió conversaciones de soporte comunitario. No se abrieron ni se usaron como evidencia del comportamiento del producto. Se eligió Carbon por documentar explícitamente los estados y acciones del componente.

### Complementos recomendados para revisar

1. **Selección con alcance visible.** Conservar Todos y Ninguno globales, además de cada categoría. Mostrar cuántos elementos se seleccionaron y un estado parcial cuando corresponda. Carbon fundamenta los estados de selección; el alcance global proviene del requisito de Jesús. El costo es reservar espacio para ese resumen, especialmente en móvil.
2. **Configuraciones de IA reutilizables.** Ofrecer acceso directo a combinaciones guardadas y mostrar cuál está aplicada. Antes de aplicar una recomendación, mostrar qué modelo y herramientas cambia y el motivo verificable. Raycast fundamenta la reutilización; las recomendaciones deterministas siguen siendo diseño propio pendiente. El costo es mantener esas combinaciones cuando cambian modelos o herramientas.
3. **Atención con contexto.** Separar avisos pendientes de intervención del historial informativo y distinguir leído, pospuesto y resuelto. Linear fundamenta la distinción entre acciones de bandeja y trabajo. Se conserva la franja con flotantes. La alternativa es una lista cronológica simple, con menos controles pero más esfuerzo para localizar pendientes.

Como complementos secundarios, VS Code aporta ampliación temporal de grupos y TickTick aporta inicio contextual y análisis del tiempo. No se incorporan automáticamente. La restauración de conversaciones, las acciones seguras de Telegram y el costo real en tokens requieren evidencia del sistema de ComandOS; una captura externa no resuelve esos contratos.

### Evidencia y alcance de esta investigación

Se inspeccionaron seis imágenes: bandeja y opciones de Linear, chat y ajustes de Raycast, grupos de VS Code y selección por lote de Carbon. También se guardó una captura de la documentación de Linear. Las capturas del navegador pertenecen a la Mac mini y se copiaron explícitamente al workspace. Son ejemplos oficiales y capturas de documentación, no pruebas de uso autenticado de esas aplicaciones.

Los intentos de descargar imágenes de TickTick y dos imágenes de Carbon directamente desde este equipo fallaron. Carbon sí se pudo capturar desde el navegador remoto. La recomendación de TickTick se apoya en su documentación, sin afirmar revisión visual completa en móvil.

La evidencia local está en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/research-complements. El inventario de fuentes está en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/research-complements/sources.json. Las imágenes se usan para revisión, no como assets del producto.

No se generaron glimpses, branding, una mascota ni HTML nuevo: el alcance pedido es buscar referencias complementarias. No hay cambio de stack, importación de componentes ni modificación de tokens visuales. Las propuestas todavía requieren evaluación dentro del mockup de ComandOS y la revisión humana ya acordada.


## Mobbin recuperado y detalle de la franja de avisos · 29 de septiembre de 2026

Jesús indicó detener la investigación hasta recuperar Mobbin. Se corrigió el curso: las fuentes oficiales anteriores se conservan como evidencia independiente, no como sustituto autorizado de Mobbin.

El cliente MCP integrado seguía fallando al inicializar. El transporte configurado, `/home/someguy/.local/bin/cc-extensions serve mobbin`, respondió al healthcheck y a un cliente MCP acotado que usa la misma autenticación compartida. Se obtuvieron sus tres herramientas y se ejecutaron tres búsquedas reales mediante `search_screens`. No se abrió otro login, se duplicaron credenciales ni se scrapeó Mobbin. La descarga inicial no seguía una redirección HTTP 302; al habilitar redirecciones se descargaron las imágenes originales desde los enlaces devueltos por el MCP. Se inspeccionaron las ocho imágenes. Cada cliente temporal se cerró al terminar. Esto recupera la investigación; no prueba que las herramientas integradas de esta conversación se hayan reconectado.

Consultas: bandeja de notificaciones web, bandeja de avisos iOS y bandeja de Linear web. Modo standard, límites 4, 3 y 1. Los resultados no se describieron a partir de metadatos. Algunos resultados son tangenciales y se descartan abajo.

| Pantalla Mobbin | Qué se vio en la imagen | Uso en esta ronda |
|---|---|---|
| [Klaviyo · helpdesk](https://mobbin.com/screens/8f7ead49-4c06-4459-b340-dbfd6a8f7834) | Lista de conversaciones junto al detalle seleccionado, acciones en la cabecera y estado Open. | Parcial. Comparar lista con vista previa en D; no copiar sus cuatro columnas ni atribuirle una bandeja de permisos. |
| [Basecamp · Latest Activity](https://mobbin.com/screens/b71d21b5-1078-44a4-adb3-9583531a6059) | Actividad por día, hora, proyecto y autor, filtros visibles y acceso inferior a Pings/New for you. | Parcial. Cronología en A y contexto de proyecto en B. El agrupado por proyecto de B es una adaptación propia. |
| [Canny · changelog](https://mobbin.com/screens/2c6b438e-5b00-44f2-aa82-e3a05ed47c67) | Publicaciones extensas, filtros Draft/Scheduled/Published y All/New/Improved/Fixed. | No. Es administración de publicaciones; no fundamenta la atención de avisos. |
| [Tana · Activity](https://mobbin.com/screens/05899178-5ec3-4a5e-831e-703561fd25ec) | Actividad reciente y columna Notifications, puntos de no leído, Mark all read y filtro Unread. | Parcial. Filas compactas y lectura separada de la acción de trabajo. No demuestra prioridades ni permisos. |
| [Mindtrip · Updates](https://mobbin.com/screens/3c451683-6c3d-4c76-be6a-0a129143ba3a) | Contador de no leídos, punto junto a cada aviso nuevo, título, extracto y hora en una lista móvil. | Sí. Contador visible y origen legible en las variantes móviles. No se copian emojis ni imágenes. |
| [LinkedIn · Notifications](https://mobbin.com/screens/2f976b2e-5bd9-4b99-ab34-12988fd7d200) | Filtros All/Jobs/My posts/Mentions sobre una lista, diferencias de fondo y puntos para avisos. | Parcial. Filtros táctiles en la franja; se evita mezclar sugerencias y publicidad con avisos operativos. |
| [Mesh · Home](https://mobbin.com/screens/8db45ea4-7352-4edf-bd49-6f68e0b8c791) | Grupos por día y menú Filter by/Dismiss All Items. | No para la acción global. Ocultar todo podría confundir lectura con resolución. En ComandOS se propone Marcar todos leídos y conservar permisos pendientes. |
| [Linear · issue](https://mobbin.com/screens/cef36326-d8ec-4c6f-acd4-a9f1e1060d33) | Detalle de un issue con propiedades y actividad; la bandeja solicitada no está abierta. | No para esta ronda. No se atribuyen patrones de inbox a esta captura. |

### Cinco propuestas dentro de la decisión ya aceptada

Todas mantienen franja inferior y aviso flotante, acceso al pane de origen, lectura independiente del permiso, ejemplo de pane cerrado, errores, carga, vacío y desconexión. Las acciones permanecen simuladas. No se conecta Telegram ni se cambia su política de entrega.

- A, Por tiempo. Lista compacta de eventos recientes. Facilita rastrear qué pasó; exige buscar los permisos entre los demás avisos.
- B, Por proyecto. Columnas en escritorio y grupos apilados en móvil. Facilita retomar un proyecto; requiere recorrer los grupos para comparar toda la actividad.
- C, Primero tu respuesta. Recomendación para evaluar: permisos pendientes separados de la actividad. La prioridad procede del tipo de evento y del estado pendiente, no de IA ni de una captura externa. En móvil se apilan las dos secciones.
- D, Vista previa. Lista y detalle en la franja. En móvil el detalle reemplaza la lista y ofrece volver. Evita cambiar de pane antes de leer, a cambio de una interacción adicional.
- E, Uno a uno. Un aviso detallado con flechas para recorrer. Hace cómodo revisar cada evento, pero da menos visión global. Es una hipótesis propia, no un patrón atribuido a Mobbin.

No se abre de nuevo la decisión sobre la ubicación de las notificaciones. Esta ronda baja un nivel al orden, densidad y acción de cada aviso. Las variantes siguen pendientes del veredicto humano. No hay mascota, branding, dependencias nuevas ni importación de componentes; se usa el HTML existente y SVG inline. El pedido explícito de grill-design autoriza las cinco variantes interactivas dentro del mismo prototipo, sin otro paso de permiso para producirlas.

Evidencia original: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-notifications. Los inventarios son /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-notifications/web-results.json, /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-notifications/ios-results.json y /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-notifications/linear-results.json. Las imágenes se usan como evidencia de revisión, no como assets de la interfaz.

Prototipo actualizado: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/prototype-v1-grill.html. Ruta de revisión: `?round=notification-detail&variant=C`. Las rutas anteriores mantienen la ronda de ubicación y su elección C.


### Veredicto: por proyecto; continuación dentro de cada grupo

Jesús eligió B, Por proyecto: "por proyecto me gusta". La ronda anterior ahora marca B como elegida y abre esa base por defecto. C, Primero tu respuesta, queda como alternativa no seleccionada.

La siguiente comparación conserva el agrupado por proyecto y contrasta lista completa, pilas por pane, pestañas de panes, avisos con detalle y filtros por tipo dentro de cada proyecto. Se reutilizan las ocho capturas de Mobbin ya inspeccionadas en esta investigación. No se hizo otra búsqueda ni se atribuyen a Mobbin las pilas de terminales, la identidad de pane o las reglas de permisos: esas adaptaciones son hipótesis propias de ComandOS pendientes del veredicto. Tana fundamenta la separación visual entre filas y lectura; Klaviyo aporta el contraste lista/detalle; Mindtrip y LinkedIn aportan contadores y filtros visibles en móvil. No se copian assets externos.

Se verificó el prototipo con Chrome remoto en la Mac mini a 1440×900, 820×1180, 390×844, 667×375 y 320×568. La limitación de navegador ocupado de la ronda anterior dejó de bloquear esta verificación. Se corrigió el selector de revisión que tapaba el pie de la app, y en pantallas bajas se usa scroll para evitar que terminal y franja se solapen. B ya está aceptada para agrupar proyectos; B de la nueva ronda, Pilas por pane, solo es una recomendación para evaluar.

Fuente actualizada: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/prototype-v1-grill.html. Registro de decisiones y comprobaciones: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/README-v1-grill.md. Capturas locales: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/notification-projects/comandos-notification-projects-mobile-20260929.png y /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/notification-projects/comandos-notification-projects-desktop-20260929.png. Proceden de archivos generados en la Mac mini y transferidos explícitamente.

## Lector de novedades de IA · 29 de septiembre de 2026

La pregunta visual es cómo leer resúmenes dentro de CommandOS conservando acceso a la terminal. Jesús pidió ver el diseño mediante grill-design. Se construyen cinco variantes en la ruta existente, con el mismo contenido ficticio y los mismos controles en escritorio y remoto. La frecuencia de las notificaciones sigue pendiente; esta ronda no la decide.

Motor: MCP Mobbin, mediante el transporte autorizado /home/someguy/.local/bin/cc-extensions. Dos búsquedas reales, web e iOS, en modo standard con tres resultados cada una. Se descargaron e inspeccionaron las seis imágenes originales. La búsqueda y las imágenes respondieron sin errores. No se reutilizaron sus contenidos editoriales como noticias ni sus imágenes como assets del producto.

| Pantalla | Qué se inspeccionó | Uso |
|---|---|---|
| [GetYourGuide](https://mobbin.com/screens/7771ec75-2458-4381-9cbe-f0cb87d4b027) | Columna de artículo con subtítulos, enlaces y un índice lateral separado de las recomendaciones. | Parcial. Ancho de lectura y jerarquía de texto. Se descarta la columna promocional. |
| [Mintlify](https://mobbin.com/screens/b409b4ea-993e-4cb5-ae9e-8aaab0558700) | Navegación de documentos junto a un artículo; conversación separada en otra columna. | Parcial. Lista y detalle en A; contexto de trabajo junto al documento en C. No se copia el editor ni el botón de publicar. |
| [Microsoft Copilot](https://mobbin.com/screens/602ae92b-9bf8-49dd-9e44-7632bab04bf0) | Lectura extensa con índice lateral, subtítulos y referencia junto al texto. | Sí. Índice y recorrido editorial en B; fuentes en el artículo. No se verifica ni reutiliza la noticia de la captura. |
| [Public](https://mobbin.com/screens/d4ff9a4c-6714-4e65-8de5-2efe0640ca77) | Lista móvil con fuente, antigüedad, titular y extracto; categorías horizontales. | Sí. Filas legibles en A y filtros desplazables. Se descartan cotizaciones y miniaturas decorativas. |
| [Perplexity](https://mobbin.com/screens/4873615d-9df3-4992-b758-d6dac2cad573) | Resumen móvil con titulares desplegables, un texto abierto y control de fuentes. | Sí. Expansión en el lugar en D y lectura breve antes del artículo. |
| [LinkedIn](https://mobbin.com/screens/9328b77e-e61b-482e-ba68-35852db74f05) | Navegación de noticias con flechas y puntos de progreso sobre publicaciones relacionadas. | Parcial. Recorrido anterior/siguiente y posición visible en E. Se descartan comentarios, reacciones y contenido social. |

Direcciones: A, Lista + lector, favorece elegir una noticia y leerla con espacio; C, Junto a la terminal, conserva el trabajo visible a costa del ancho; D, Temas desplegables, favorece explorar categorías a costa de más aperturas. A es la recomendación inicial para lectura cómoda desde el celular. Ninguna está elegida por el usuario.

Stack: HTML, CSS y JavaScript del prototipo existente; iconos SVG propios, fuentes del sistema y contenido en memoria. Las cinco variantes sustituyen los glimpses separados porque el usuario pidió expresamente la ronda interactiva dentro de CommandOS. No se introduce mascota, branding ni una dependencia de renderizado. El pequeño parser de Markdown se limita al contenido fijo del ejemplo y no constituye una solución para renderizar noticias externas en producción.

Inventario de Mobbin: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-news-reader/results.json. Imágenes originales: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-news-reader. Capturas del mockup A, B, C, D y E en escritorio y móvil: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/news-reader. Todas las capturas del mockup se generaron en Chrome de la Mac mini y se transfirieron explícitamente a este equipo.

Prototipo: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/prototype-v1-grill.html. Ruta: https://nodo-01.tail63a117.ts.net:8444/prototypes/prototype-v1-grill.html?round=news-reader&variant=A. El registro de verificación está en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/README-v1-grill.md. Tokens y alcance: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/DESIGN.md. Prueba visual: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/proof.html.

### Veredicto: edición con terminal opcional

Jesús eligió B y pidió poder ver la terminal. Se conserva la edición continua y se agrega un control para compartir el espacio. Se reutilizan las seis capturas verificadas; no hubo otra búsqueda. El contexto de trabajo junto al documento observado en Mintlify sirve como referencia parcial. La terminal debajo del lector en móvil, el separador y la conservación del ancla de lectura son decisiones propias del prototipo, pendientes de la revisión de ese detalle.

Ruta con ambas vistas: https://nodo-01.tail63a117.ts.net:8444/prototypes/prototype-v1-grill.html?round=news-reader&variant=B&terminal=1. Evidencia visual local: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/news-reader/comandos-edition-terminal-desktop.png y /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/news-reader/comandos-edition-terminal-mobile.png. Las comprobaciones y el alcance están en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/dash/prototypes/README-v1-grill.md.


### Corrección de Jesús: terminal siempre a la izquierda

La terminal queda a la izquierda y la edición a la derecha. Esta indicación sustituye la propuesta anterior de terminal a la derecha o debajo. Hasta 560 px se propone mantener ambas vistas con ancho completo y navegación horizontal mediante botones o desplazamiento. Es una adaptación del prototipo para revisión, sin una nueva búsqueda de referencias. Las seis capturas de Mobbin ya inspeccionadas siguen siendo la base del lector. La ruta de revisión conserva B y `terminal=1`.

Veredicto del 2026-09-29: Jesús aprobó el mockup corregido, incluida la navegación móvil descrita. Queda elegida la edición continua con terminal opcional a la izquierda. La frecuencia de avisos sigue pendiente.

### Aviso breve en Android con preview

El usuario pide un aviso breve y una preview. Se agrega una simulación de contenido expandible sobre la ronda de notificaciones existente, conservando la agrupación por proyecto. No se hace una nueva búsqueda de Mobbin ni se atribuye a sus capturas una apariencia nativa de Android. La presentación del sistema es aproximada; las capacidades y límites de Web Push proceden de las fuentes oficiales ya verificadas en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/mobile-push-research.md. La propuesta concreta y sus pruebas están en /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/android-push-preview.md.

## Barra lateral izquierda · 29 de septiembre de 2026

La pregunta es cómo encontrar y retomar proyectos, sesiones y panes. Se mantienen el marco del producto, los mismos datos de ejemplo y los controles en escritorio y móvil. Las cinco estructuras dentro de la app responden al pedido de grill-design; sustituyen propuestas visuales separadas para que Jesús pueda probarlas en contexto. No se agrega branding, mascota, biblioteca ni tipografía.

Se consultó MCP Mobbin mediante /home/someguy/.local/bin/cc-extensions. El primer cliente no pudo importar el SDK de MCP y se detuvo antes de consultar el servicio. Se recuperó usando el runtime ya instalado en /home/someguy/.local/share/comandos/extensions-venv/bin/python. Se pausó el diseño dependiente hasta obtener resultados reales; no se sustituyó Mobbin por capturas inventadas ni búsqueda en el navegador.

Se ejecutaron dos búsquedas `search_screens` en modo standard, con tres resultados cada una: web para barras de proyectos con jerarquía, favoritos y búsqueda; iOS para navegación de proyectos y recientes. Se descargaron e inspeccionaron las seis imágenes originales. Las referencias móviles aportan patrones de navegación, no una afirmación de comportamiento nativo de Android.

| Pantalla | Observación en la imagen | Uso en la propuesta |
| --- | --- | --- |
| [Shopify · editor](https://mobbin.com/screens/012c4dd5-9848-48f4-aa91-83515555ee60) | Explorador jerárquico, grupos plegables y riel estrecho de iconos. | Parcial para A y C. En ComandOS las entidades son proyecto, sesión y pane; no archivos de una tienda. |
| [Dropbox Dash](https://mobbin.com/screens/e049b7be-84e1-41bb-bccb-3e83eae2b21d) | Carpetas indentadas junto al documento y fila seleccionada visible. | Parcial para la pertenencia y la selección única. No se importan sus acciones de documentos. |
| [v0 · web](https://mobbin.com/screens/cd2fab36-5aa9-4ea9-97cb-e4fc0d7bb08b) | Conversación y explorador separados del área de edición. | Parcial para separar navegación, contexto y chat. Se descartan tres columnas permanentes por su costo de ancho. |
| [v0 · iOS](https://mobbin.com/screens/e9617000-59b8-4777-802a-1b34fcb3bc25) | Menú que separa Website, Chat y Project y contiene una acción de favorito. | Referencia parcial del alcance de las acciones. La captura no demuestra una barra de proyectos. |
| [Perplexity · Projects](https://mobbin.com/screens/6ebe52a4-df58-438d-aa38-f3a8ac54da75) | Proyectos fijados, búsqueda y acciones por fila. | Favoritos y búsqueda en D; filas táctiles. No se copian emojis ni assets. |
| [Lifesum · Favorites](https://mobbin.com/screens/ff1fa58b-09db-4b96-9708-6ca980be542d) | Favoritos de recetas, categorías y ordenación. | Descartada. No aporta una jerarquía útil para sesiones de terminal. |

La adaptación a sesiones, el inspector del pane, los recientes basados en selección explícita y la conservación del borrador son hipótesis propias de ComandOS. Las referencias no prueban esas capacidades. A es una recomendación para evaluar y no un veredicto del usuario.

Inventario de fuentes: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-sidebar/results.json. Imágenes originales: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/mobbin-sidebar. Se conservan como evidencia de investigación, no como assets de la interfaz.

Capturas del mockup: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/shots/sidebar. Se generaron con Chrome remoto en la Mac mini y se transfirieron explícitamente. Alcance y pruebas: /home/someguy/codebase/0xJesus/ComandOS/.worktrees/comandos-v1-grill/design/sidebar-grill.md. Ruta de revisión: https://nodo-01.tail63a117.ts.net:8444/prototypes/prototype-v1-grill.html?round=sidebar&variant=A.

## Barra de comandos y terminales rápidas · 29 de septiembre de 2026

Jesús rechazó la primera versión de la ronda 2 por fea y sin referencias: exige direct manipulation, arte pixel de videojuego, iconos bonitos y una UX cómoda, como en las rondas anteriores. Se rehizo el prototipo dentro del marco del producto con los packs pixel ya inventariados en dash/prototypes/assets/pixel-packs y dos búsquedas reales en MCP Mobbin (modo standard), con imágenes abiertas e inspeccionadas.

| Pantalla | Observación en la imagen | Uso en la propuesta |
| --- | --- | --- |
| [Mintlify · paleta](https://mobbin.com/screens/7c7ad31f-9dfe-4be7-83d7-6002fe31d4d0) | Paleta con búsqueda arriba, grupos con etiqueta y una línea de explicación bajo cada comando; pistas de teclado al pie. | Sí: comando + explicación corta en cada fila y grupos con cabecera. |
| [Devin · paleta](https://mobbin.com/screens/274435b9-f420-4a7c-ab68-82514c1419f9) | Secciones Actions / Navigation / Settings, atajos a la derecha, barra lateral de sesiones detrás. | Sí: agrupación por intención; el atajo de teclado se sustituye por opciones de argumento. |
| [Higgsfield · menú /](https://mobbin.com/screens/2f829612-e921-42c0-ba5c-2fd60fd97d77) | "Press / for commands": menú oscuro flotante junto al texto, iconos por fila, tono oscuro cercano al producto. | Sí para B: la tecla / abre el libro de comandos sobre la terminal. |
| [fal · paleta](https://mobbin.com/screens/d8e46a56-c6c6-4db3-b85f-83bd6cedc01f) | Sugerencias con icono, título y descripción de dos líneas; pie que invita a escribir. | Parcial: densidad de dos líneas por comando. |
| [v0 · paleta](https://mobbin.com/screens/29db691c-e7fb-4ee3-9f4b-e16c4970b92a) | Comandos y recientes mezclados con marcas de tiempo. | Descartada: mezclar historial con comandos confunde el catálogo verificado. |
| [Perplexity · atajos](https://mobbin.com/screens/b0a63cb0-ff2e-49a9-90ab-b4dff7f8084d), [Cofounder · atajos](https://mobbin.com/screens/2f5bbe84-7929-4d5e-8a3e-8a899c4c89a3), [Google Ads](https://mobbin.com/screens/bcb339b6-4b81-48d8-80bb-1de71a8ccc61) | Listas de atajos por sección; navegación con teclas. | No: son referencias de atajos, no de ejecución de comandos. |
| [MagicPath · workspace](https://mobbin.com/screens/0b864c09-add9-4227-9b1c-6d40a9e3e80d) | Panel de código a la derecha, chat a la izquierda, lienzo central, todo oscuro. | Parcial para D: panel secundario acoplado al workspace. |
| [Shopify · búsqueda en editor](https://mobbin.com/screens/34965107-7534-4e63-a03c-4da04f68e834), [Mintlify · editor](https://mobbin.com/screens/8f94c65b-0206-4633-a515-261aee5c928e) | Barra lateral de resultados y navegación de archivos junto al editor. | No: no muestran una terminal integrada; no se citan como prueba de un cajón de terminales. |

Arte pixel reutilizado con sus licencias ya registradas: Shikashi (libro de hechizos para Comandos, cerebro para modelo, ciclo para conversación, llave para cuenta, mapa para contexto, grilletes para Cadenas, rayo para terminal rápida, hoguera para arranque, mano lanzando magia para Enter), karsiori (poción burbujeando en el pane seleccionado, cofre dorado al completar una cadena), Zoedoz (reloj de arena mientras corre una cadena). Los iconos en la barra son celdas de la hoja original a 32 px, sin redibujar.

Direct manipulation implementada en el mockup: clic inserta el comando letra por letra sin Enter; arrastrar un comando a cualquier pane lo escribe ahí y lo selecciona; arrastrar a Cadenas lo añade como paso; los pasos se reordenan arrastrando; el separador de A cambia la altura. Mobbin no aporta pantallas de terminal integrada con tabs; las composiciones C y D son hipótesis propias. Verificado en la Mac mini a 1440×1000 y 390×844.

## Ficha viva y panel de sesiones: piel de ComandOS, 2026-11-03

Brief: el contenido del panel de sesiones y de la ficha por sesión ya está decidido por la investigación cognitiva (qué quiere el agente, cuánto espera, en qué ibas, cronología, etiquetas). Jesús rechazó la piel de las rondas 1–7 (fondos rojo/verde/ámbar, chips, losetas de colores) y pidió basarse en la investigación y en componentes UX/UI probados. Motor: MCP `mobbin` (app/SaaS). Mobbin respondió con imágenes reales en las tres búsquedas; se examinaron las capturas, no solo los metadatos. Maqueta: https://claude.ai/artifact/U2gGcJjQH85Xtq4PCg4afz (ronda 8, variantes P1–P5), verificada con capturas en el Chrome remoto de la Mac mini (`cc-extensions serve chrome-bg` + `cc-browser-expose`).

| Fuente | Qué se revisó | Uso |
|---|---|---|
| [ComandOS, ventana real](../dash/index.html) | Captura de la app el 3-nov: losetas grises nombre · modelo en la cuadrícula de sesiones, tarjetas de Consumos con etiquetas mono en mayúsculas, un acento, cabecera con botones 3D. | Sí: tokens y componentes base de P1–P5 (`--panel #121722`, `--text #EAF0FB`, acento `#8B7CFF`, Ubuntu Sans / Ubuntu Sans Mono). |
| [DESIGN.md del repo](DESIGN.md) | Tema oscuro, acento, 44 px en acciones, tarjetas para resumen de sesiones. | Sí: regla "un acento"; la ficha usa la tarjeta de Consumos. |
| [Linear · Inbox](https://mobbin.com/screens/18c6955a-c42f-4cc1-9aa2-0276456720fb) | Bandeja: punto de no leído, título en negrita, subtítulo gris, tiempo a la derecha, sin color de fondo. | Sí: fila de P2/P3/P5 (punto rojo solo si te espera, qué quiere en gris, espera a la derecha). |
| [Gumloop · Inbox "needs your input"](https://mobbin.com/screens/a0a76d11-fd8c-4720-9194-d6d5e43c8b0e) | Lista de agentes que piden entrada: nombre + "needs your input" + hace cuánto; pestañas Inbox / Resolved. | Sí: agrupación Te necesitan / Terminaron y texto "qué quiere" literal. |
| [Plane · Inbox](https://mobbin.com/screens/849ec3f0-a1f0-4699-85eb-f895f7330036) | Lista con conteo en badge gris, filas planas. | Sí: badge gris del conteo en la pestaña Sesiones (no rojo). |
| [Relevance AI · Task timeline](https://mobbin.com/screens/12228e33-2d48-435e-9027-f0c3d0f4e5f0) | Lista agrupada por Today / Last 3 days con hora a la derecha; detalle con cronología vertical sin colores. | Sí: cronología de la ficha (hora en mono gris, último paso en blanco). |
| [Jira · Notifications](https://mobbin.com/screens/f7eb5777-1125-4f47-bdcc-39931e19cfeb) | Punto azul de no leído como única señal de color en una lista densa. | Sí: confirma "una sola señal"; el punto es rojo en ComandOS porque ya es el color de "pide permiso". |
| [ClickUp · Inbox](https://mobbin.com/screens/f1374e47-4168-46b0-baab-445916244ac6) | Pestañas Primary / Other / Later / Cleared con conteos. | Parcial: idea de conteos por grupo; no se adoptan pestañas. |
| [Asana · Inbox summary](https://mobbin.com/screens/ce717ee0-8445-49d2-9dda-227a396cef9a) | Resumen "requires your action" arriba de la lista. | No: el resumen duplica lo que ya dice la cabecera (N te necesitan). |
| [Wrike](https://mobbin.com/screens/a2b51485-af81-4c80-8f06-b357e0fe5bda), [Remote](https://mobbin.com/screens/5e916e4a-4c86-4c06-a455-9a8d1785603d), [Ditto](https://mobbin.com/screens/caef87ed-4f3a-4224-83fb-3262ebcc593d) | Bandejas con tarjetas de aviso y onboarding. | No: demasiado ruido para un panel de 254 px. |
| [Relevance AI · Agents table](https://mobbin.com/screens/1fdb02be-0b40-47d4-8bd4-ab77268c5f9f), [Lindy · My agents](https://mobbin.com/screens/f2ddc92e-973c-489d-b92c-72de8fb5adde) | Tablas de agentes con columnas y última ejecución. | Parcial: base de P4 (tabla Sesión · Qué quiere · Espera). |
| PlayAI, Plain, Langdock, Lemni, Juicebox, Wix, Gorgias (Mobbin, misma búsqueda) | Páginas de agentes con tarjetas grandes y marketing. | No: son vistas de catálogo, no de operación. |
| [Airtable](https://mobbin.com/screens/4e6059d4-ad78-48ec-8275-722ec190e3ac), [Wrike detalle](https://mobbin.com/screens/866c9861-876e-4cbc-9219-0dad94db8520), [ClickUp detalle](https://mobbin.com/screens/36758b9b-f6a3-4a8b-b4d2-91c373742924), [Slack lists](https://mobbin.com/screens/328deda6-5c30-4d8a-ba1a-2410ac5f11c4), [Unify](https://mobbin.com/screens/243c3a1b-efd7-44a4-8acc-dadc41f1de7c) | Paneles laterales de detalle con propiedades y comentarios. | Parcial: etiquetas en mono mayúsculas sobre el valor (IBAS EN, TE ESPERA); se descartan comentarios y propiedades vacías. |

Direcciones con nombre:
- **Robar: Bandeja Linear** (punto + negrita + gris + tiempo). Es la fila de P2/P3/P5.
- **Evitar: semáforo** (fondos por estado, chips de etiqueta, tres colores). Es lo rechazado en las rondas 1–7.
- **Recomendada: P5 Bandeja + cuadrícula**: filas con texto solo en Te necesitan / Terminaron; debajo, la cuadrícula actual nombre · modelo para las que trabajan o están libres; filtro de etiqueta como un renglón de texto; etiquetas editables en la ficha.

Stack: todo son tokens y clases ya existentes en `dash/workspace.css` (tarjeta, pill, mono mayúsculas); no se añade librería. No pegar el CSS de la maqueta tal cual: reimplementar con los tokens del tema.

Glimpses y mascota: saltados; la maqueta es la propia app con datos de ejemplo y el usuario está eligiendo entre P1–P5 en la ronda de grill.

Archivos del run: maqueta `ficha-viva-grill.html` (artifact anterior), capturas en el scratchpad de la sesión (`shots7/`); DESIGN.md se actualiza cuando Jesús elija la P.

### Ronda 9 (2026-11-03): sin cubetas de estado

Jesús rechazó «te necesitan / terminaron / trabajando / libres»: «no creo que podamos mapearlo bien, no quiero que esté tan hardcodeado». Pidió mostrar bien solo Claude Code y Codex. Fuente de esta ronda: el propio repo, no Mobbin.

| Fuente | Qué se miró | Uso |
|---|---|---|
| `crates/comandos-runtime/src/hooks/claude.rs` | UserPromptSubmit → working; Notification (permission_prompt / idle_prompt, pregunta leída del transcript) → waiting con `detail` y `options`; Stop → done con el último texto del turno; SessionEnd borra el estado | sí: cada fila muestra ese evento literal |
| `crates/comandos-runtime/src/hooks/codex_hooks.rs`, `codex.rs` | Codex hooks: UserPromptSubmit, Stop, PermissionRequest; notify `agent-turn-complete` con `last-assistant-message` | sí |
| `crates/comandos-server/src/dash/native/states/cards.rs` | la clasificación actual: `DONE_GRACE_SECS` 600, `ZOMBIE_WAITING_GRACE_SECS` 86 400, regex `Working (`, `esc to interrupt`, `Do you want / Allow` + `❯ 1.`; estado por defecto `idle` → `dead` | parcial: solo la regex del diálogo de permiso (Q4) y la última línea del pane como respaldo |

Dirección: una lista por hora del último evento; punto rojo = Notification o PermissionRequest sin un prompt del usuario después («aviso sin responder»); filtro por etiqueta (gustó). Recomendada Q5: filas con el evento literal mientras quepan, el resto losetas con su hora; si una sesión no manda hooks, su fila muestra la última línea del pane.

### Ronda 10 (2026-11-03): cuál es «la última línea de la terminal»

Verdicto de Jesús sobre la ronda 9: «última línea de la terminal» (Q4). Antes de bajar un nivel capturé los panes reales con `tmux capture-pane -p -S -14` (solo lectura, socket por defecto, sin formatos de tiempo) de 22 panes con `claude`, `codex` o `node`.

| Fuente | Qué se miró | Sirvió | Qué se tomó |
|---|---|---|---|
| Panes reales (capture-pane, 22 panes, 3-nov 22:50) | Últimas 8 líneas no vacías de cada uno | sí | Estructura del pie de pantalla: Claude Code termina SIEMPRE en la barra «⏵⏵ bypass permissions on (shift+tab to cycle) · ⧉ N» (a la derecha «✔ Update installed · /restart to apply» o «new task? /clear to save NNNk tokens»); encima la caja «❯» entre dos reglas; encima de la caja la cuenta del TUI «✻ Crunched for 11s · done 10:40 a.m.» o «✽ Swirling… (53s · ↓ 2.7k tokens)» y, si hay, «※ recap: …». Codex termina en «? for shortcuts · ⚠ 7 warnings · f2 to view», encima «GPT-6-Astra high · ~/ruta · título», encima «› Ask Codex to do anything», encima «Worked for 1m 6s • 3:02 PM» y «↳ Recap: …». Textos usados tal cual en la maqueta (con «⏵⏵»→«▸▸», «⎿»→«└», «⧉»→«▣» porque la fuente del Chrome remoto no los tiene). |
| `crates/comandos-server/src/dash/native/states/cards.rs` (`claude_busy`, `codex_working`) | Cómo lee hoy ComandOS el pane | parcial | Ya usa `capture-pane -S -12/-14`; la ronda 10 propone mostrar la línea en vez de clasificarla. Regex de menú numerado («❯ 1.») como único detector del punto rojo. |

Dirección: cinco definiciones de «última línea» sobre la estructura Q5/Q4 (filas con texto mientras quepan, losetas con nombre para el resto, filtro por etiqueta): R1 la barra literal (inútil, se muestra para verlo), R2 la línea encima de la caja, R3 recap del propio agente si está en pantalla y si no R2 (recomendada), R4 tres líneas en mono, R5 R2 + lo escrito en la caja sin enviar. El ajuste puntúa combinaciones (fila de dos líneas 2, de una 1.2, cuadrícula de 4 columnas −0.5). Verificado con capturas en escritorio (10/30/60/120) y celular (30). Artifact v14.

### Ronda 11 (2026-11-03): la misma tarjeta para todas

Verdicto sobre la ronda 10: R3 (recap si está en pantalla; si no, la línea encima de la caja), pero «no quiero que estén diferentes las cards del lado izquierdo; minimalista pero lo suficientemente informativa». Se elimina la mezcla filas-con-texto + losetas. Cinco estilos de tarjeta uniforme con el mismo contenido: U1 tarjetas iguales de dos líneas (recomendada), U2 lista sin cajas con línea fina (patrón Linear/Things), U3 cuadrícula de dos columnas con texto a dos líneas, U4 tabla densa de una línea por sesión (punto · nombre · línea · hace), U5 nombre en negrita y texto tenue sin palabra clave. Regla de densidad: se elige el primer modo (de más a menos texto) en el que caben TODAS las sesiones con la misma tarjeta; nunca dos formas a la vez. Hallazgo verificado con capturas: con 30 sesiones en el panel de 254 px solo la tabla densa (filas de ~17 px) conserva texto; las tarjetas de dos o una línea caen a solo nombre en dos columnas. Con 60, tres columnas de nombres; con 120, iniciales. Artifact v15.

### Ronda 12 (2026-11-03): la tabla, cómoda de leer y operar

Verdicto sobre la ronda 11: la tabla (U4), «pero más cómoda de leer y operar». Medido en el Chrome remoto: el panel izquierdo ofrece 570 px útiles; 30 filas caben solo si cada una mide ≤ 17.5 px (con 18.2 px se desborda 14 px y la tabla cae a solo nombres). Por eso la comodidad no puede venir de filas más altas sin scroll. Cinco variantes: T1 columnas con aire (cebra en vez de líneas, nombre fijo 100 px, barra azul en la activa, fondo al pasar el cursor), T2 letra proporcional para la línea de la terminal (≈30 % más caracteres en el mismo ancho), T3 tiempo compacto (14m, 2h) que cede ~25 px al texto, T4 operable (T2 + acciones en la fila al pasar el cursor o en la activa: ⏎ abrir; en diálogo 1 Sí · 2 Siempre · 3 No; y línea de teclas ↑↓ ⏎ 1 2 3 /; en celular acciones siempre visibles) recomendada, T5 filas altas (26 px, 40 px en celular) con encabezado y scroll, única que rompe «todas a la vista». Trampa encontrada: `font:` abreviado resetea line-height y subió las filas a 20 px; se fija `font:11.5px/1.1`. Artifact v16.

### Ronda 13 (2026-11-04): con aire, con scroll, y la vista de hoy a un toque

Verdicto sobre la ronda 12: «no me gusta ya ninguna, muy pegado el contenido; que haya scroll si será necesario; y tendremos que tener forma de también seguir viéndolo COMO está hoy». Se abandona la regla «todas a la vista sin scroll» para el panel de sesiones: filas con aire y scroll. En todas las variantes, conmutador Lista · Hoy en la cabecera del panel; «Hoy» es la cuadrícula de losetas nombre · modelo actual (misma lógica que P1 de la ronda 8) sin cambios. Variantes: S1 tabla con filas de 30 px (recomendada), S2 dos líneas por sesión separadas por línea fina, S3 tarjetas con aire (las más parecidas a las actuales con la línea de la terminal debajo), S4 cuadrícula de hoy arriba como mapa y lista con scroll debajo, S5 diálogos abiertos fijos fuera del scroll. Artifact v17.

### Ronda 14 (2026-11-04): cinco conceptos nuevos

Verdicto sobre la ronda 13: «ninguna me gusta». Pregunté qué fallaba: el texto de cada fila, la forma de las filas y la piel; «necesito que me propongas algo increíble», «cómodos de usar, fáciles de operar». Se empieza de cero en forma, piel y contenido.

| Fuente | Qué se miró | Sirvió | Qué se tomó |
|---|---|---|---|
| [Devin · sesiones](https://mobbin.com/screens/92e0222c-8ba8-4abe-bc18-3ddef5b2e357) | Lista lateral de sesiones de agente | sí | Título = la tarea, subtítulo corto con el resultado, punto azul de no leído |
| [Front · bandeja + atajos](https://mobbin.com/screens/6b6a701f-8589-415c-9c52-c91d4feade1c) | Bandeja con vista previa y panel de atajos | sí | Vista previa de una línea, operación por teclado |
| [Superhuman · atajos](https://mobbin.com/screens/3ae6bf95-6bbd-4232-b045-3160b748393c) | Mapa de teclas | sí | E = listo, ⏎ abrir, flechas |
| [Plain · hilos](https://mobbin.com/screens/51f96c92-d0ba-4d47-afa7-7472885a1407) | Secciones por estado, filtros arriba | parcial | Filtros arriba (Todo, Snoozed) → chips Todas · Sin leer · Permisos · Trabajando |
| [Graphite · bandeja de PR](https://mobbin.com/screens/b4996a23-98e4-4c42-9b4e-e06d24fbad59) | Secciones plegables | no | Demasiado denso para 254 px |
| [Notion Mail](https://mobbin.com/screens/8da56c96-c129-4f81-9aa0-6f370056afa7), [Plain](https://mobbin.com/screens/51f96c92-d0ba-4d47-afa7-7472885a1407), Tana, ElevenLabs, Lindy, Gumloop, Lemni, Intercom, Base44, Pipedrive, Wix, Gorgias | Paneles de agentes y bandejas | no | Paneles de configuración, no de operación en vivo |

Contenido nuevo: se deja de mostrar la línea cruda del TUI. Cada sesión muestra lo que el agente respondió de verdad (filtrando «● Compacted…», «└ …», «✻ Crunched for…», «Worked for…», «● Task … running», «⚠ …») y lo que tú le pediste (el último prompt). Estado tomado del texto literal de la pantalla: menú numerado = permiso, «esc to interrupt» / «Working (» = trabajando, «usage limit reached» = límite. «Sin leer» lo decide la app (cambió desde la última vez que la abriste), no una heurística.

Conceptos: D1 Chats (lista tipo WhatsApp: avatar con color por proyecto y ✻/◎ por agente, frase real, hora, contador de nuevos, «escribiendo…», Sí · Siempre · No en la fila) recomendada; D2 Bandeja (tipo Superhuman: lo que respondió + «Tú: …», E marca leído); D3 Cola (una tarjeta grande con lo siguiente que te toca + lista compacta); D4 Pantallas en miniatura (Mission Control); D5 Actividad (timeline Te espera · Ahora · Última hora · Hoy). Todos con scroll, chips de filtro y el conmutador a «Hoy» (losetas actuales). Verificado con capturas en escritorio y celular. Artifact v18.

### Ronda 15 (2026-11-04): tus losetas arriba, tarjetas sin botones abajo

Verdicto sobre la ronda 14: «ya nos vamos entendiendo mejor»; quitar los botones de acción de las tarjetas; dejar arriba las losetas como se ven ahora, pero mejoradas. Cambios: los botones Sí · Siempre · No y Responder · Listo · Saltar salen de las tarjetas (se responde en la ficha de la derecha). Arriba de todas las variantes van las losetas de hoy en 3 columnas, mejoradas: franja de color por proyecto en el borde izquierdo (en celular, avatar), punto rojo = permiso, punto azul = nuevo, aro que late = trabajando, la activa resaltada; tope del 40 % del alto con scroll propio y desvanecido abajo. El selector compara «Arriba: mejorada / tal cual». Abajo: E1 chats (recomendada), E2 bandeja, E3 solo lo pendiente, E4 pantallas en miniatura, E5 actividad. Primer intento con avatar dentro de la loseta descartado: en 254 px dejaba 2 letras del nombre. Verificado con capturas en escritorio (30 y 60) y celular. Artifact v19.

### Ronda 16 (2026-11-04): la barra de sesiones de arriba, como hoy

Verdicto sobre la ronda 15 (molesto): «conservar» se refería a la barra de sesiones de arriba de la app, no a losetas dentro del panel izquierdo. Captura de la ventana real de cc-app (`import -window`, sin clics): la barra superior ocupa todo el ancho sobre la terminal, 4 columnas, cada loseta «nombre · modelo» (con «+1/+2» de panes extra), la activa con borde, orden fijo. El panel izquierdo tiene Archivos · Consumos y la terminal lateral. La maqueta se rehízo con esa disposición: barra arriba sobre terminal y ficha, mismo orden fijo (las losetas no se mueven), mejorada con franja de color por proyecto y punto solo cuando hay algo (rojo permiso, azul nuevo, aro trabajando); «Arriba: tal cual» la replica sin cambios. Las tarjetas nuevas sin botones van en la pestaña Sesiones del panel izquierdo: F1 chats (recomendada), F2 bandeja, F3 lo que te toca, F4 pantallas, F5 actividad. Artifact v20.

### Ronda 17 (2026-11-04): el feed de actividad a detalle

Verdicto sobre la ronda 16: «barra de arriba + actividad me gusta» (F5). Se baja un nivel al feed. Constantes en las cinco: «Te espera · N» fijo arriba fuera del scroll; hover enlazado (pasar el cursor por un evento ilumina su loseta en la barra de arriba y al revés, verificado en el Chrome remoto con 2 elementos resaltados); grupos Ahora · Última hora · Hoy; filtros Todas · Sin leer · Permisos · Trabajando. Variantes: G1 lo último de cada sesión; G2 todo lo que pasó, incluidos tus mensajes («le escribiste»); G3 lo último + «▸ 1 evento antes» plegado (abierto en la sesión activa); G4 pregunta y respuesta («Tú: …» sobre la respuesta del agente) recomendada; G5 una línea por evento. Artifact v21.

## Ronda 18 · color sin semáforo (4-nov-2026)

Veredicto de la ronda 17: G4 (pregunta y respuesta). Jesús rechazó los colores de semáforo («no tiene sentido»): rojo = permiso, azul = nuevo, aro = trabajando, verde = listo, más un tono por proyecto. Un mismo color decía dos o tres cosas, y el rojo sonaba a error.

Cinco reglas, todas sobre G4 + barra de arriba, con la leyenda en pantalla:

| Opción | Regla | Te espera | Sin leer | Trabajando |
|---|---|---|---|---|
| K1 (recomendada) | El violeta solo cuando ComandOS te llama, lo mismo que suena | Loseta violeta tenue + punto | Punto blanco + negrita | Aro gris que gira |
| K2 | El color es el proyecto, nunca el estado | Más clara + punto blanco | Punto blanco | Aro gris |
| K3 | Cero color, brillo = importancia | Loseta invertida | Negrita | Aro gris |
| K4 | Palabras, no luces | PERMISO | NUEVO | TRABAJA |
| K5 | Un tono, más intenso = más urgente | Violeta sólida | Punto violeta | Aro violeta tenue |

En todas, la sesión abierta lleva borde blanco, no violeta, para que el violeta no tenga dos significados. Los colores del contenido de la terminal no se tocan.

## Ronda 19 · la loseta de arriba (4-nov-2026)

Veredicto de la ronda 18: K5 (un solo violeta, más intenso = más urgente). Cinco losetas para la barra de sesiones de arriba. Todas miden 27 px como hoy (medido en el navegador remoto: barra de 260 px con 30 sesiones en las cinco), así que la terminal no pierde altura. El orden sigue fijo.

- B1 Como hoy + marca K5.
- B2 El tiempo («14 min», «2m 14s») reemplaza al modelo cuando hay algo que ver.
- B3 Glifo ✻/◎ antes del nombre; el modelo sale de la loseta.
- B4 (recomendada) B1 + tarjeta flotante al pasar el cursor con lo de G4; en el celular, al mantener presionado.
- B5 «Trabajando» como línea violeta que recorre la base de la loseta.
