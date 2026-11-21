# ComandOS para Android

Cliente nativo (Flutter, solo Android) del panel de ComandOS por Tailscale.
Estructura, reglas y puntos de extensión: [ARCHITECTURE.md](ARCHITECTURE.md).

## Compilar la versión final

```
export PATH=$HOME/flutter_global_3_27_4/flutter/bin:$PATH
flutter analyze && flutter test
flutter build apk --release --split-per-abi --obfuscate --split-debug-info=build/symbols --tree-shake-icons
```

El APK para teléfonos es `build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`.
Guarda la carpeta `build/symbols/` de cada versión: sin ella no se pueden leer
los errores de esa versión (`flutter symbolize`).

La firma es la llave de depuración local de esta computadora (instalación
personal, no tienda). Para publicar en otro lado hace falta una llave propia.

## Cómo instalarla y emparejarla

1. **Tailscale en el teléfono.** Instala Tailscale, entra con la misma cuenta
   que tu computadora y déjalo en «Connected».
2. **ComandOS remoto encendido en la computadora.** En ComandOS abre el panel
   «Remoto» (o corre `comandos mobile on`). Verás un código QR y un enlace del
   tipo `https://<tu-equipo>.<tu-red>.ts.net/?token=<TOKEN>`.
3. **Instala el APK.** Pásalo al teléfono (cable, Drive, lo que uses), ábrelo y
   permite «Instalar apps desconocidas» para esa app cuando Android lo pida.
   Con cable y depuración USB también sirve:
   `adb install -r app-arm64-v8a-release.apk`.
4. **Empareja.** Abre ComandOS en el teléfono → «Escanear código QR» y apunta al
   código de «Remoto» (pide permiso de cámara la primera vez). Si no puedes
   escanear, copia el enlace y pégalo en «O pega el enlace».
5. **Avisos.** Justo después te pregunta si quieres avisos en segundo plano. Si
   dices que sí, Android te pide permiso de notificaciones y verás un aviso fijo
   y discreto («ComandOS · atento a tus sesiones») mientras está activo. Lo
   puedes apagar o encender en Ajustes.

Usa siempre el nombre `…ts.net`, nunca la IP de Tailscale: ComandOS la rechaza.
Si cambias el token en la computadora, en el teléfono ve a Ajustes → «Volver a
vincular» y escanea el código nuevo.

Si los avisos llegan tarde, en Ajustes → «¿Los avisos llegan tarde?» abre los
ajustes de la app y pon la batería en «Sin restricciones».

## Permisos

Cada permiso está justificado en `android/app/src/main/AndroidManifest.xml`:
internet y estado de red (hablar con tu computadora), cámara (QR, se pide al
escanear), micrófono (dictado, se pide al usarlo), notificaciones y servicio en
primer plano (avisos en segundo plano, solo si los activas) y arranque del
teléfono (reactivar los avisos tras reiniciar, solo si estaban activos).
