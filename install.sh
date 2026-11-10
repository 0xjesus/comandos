#!/usr/bin/env bash
# Instalador de ComandOS. Enlaza los binarios y hooks a su lugar, registra los
# servicios systemd de usuario y la app de escritorio. Idempotente.
set -euo pipefail
REPO="$(cd "$(dirname "$0")" && pwd)"
BIN="$HOME/.local/bin"
HOOKS="$HOME/.claude/hooks"

# shellcheck source=lib/platform.sh
. "$REPO/lib/platform.sh"
# shellcheck source=lib/retire-telegram.sh
. "$REPO/lib/retire-telegram.sh"

# Instala los paquetes que ComandOS necesita en Ubuntu WSL (auto-fix con confirmación).
_cc_wsl_install_deps() {
  sudo -v || { echo "sudo requerido para instalar dependencias."; exit 1; }
  local pkgs
  read -ra pkgs <<< "$(cc_wsl_ubuntu_packages)"
  apt_install_confirmed "${pkgs[@]}" || {
    echo "Instala manualmente y vuelve a correr ./install.sh"
    exit 1
  }
}

# Habilita systemd en WSL editando /etc/wsl.conf (con confirmación).
# Sale con exit 0 después de escribir — el usuario debe correr `wsl --shutdown`.
_cc_wsl_enable_systemd() {
  echo "⚠  systemd no está corriendo en tu WSL."
  echo "   Sin systemd, cc-dash y cc-notifyd no pueden autoarrancar."
  ask_yn "¿Habilito systemd editando /etc/wsl.conf?" || {
    echo "Ok, hazlo a mano y vuelve a correr ./install.sh"
    exit 1
  }
  if [ -f /etc/wsl.conf ] && grep -q '^\s*systemd\s*=\s*true' /etc/wsl.conf; then
    echo "  /etc/wsl.conf ya tiene systemd=true — algo más está mal."
    echo "  Verifica con: sudo systemctl is-system-running"
    exit 1
  fi
  if [ -f /etc/wsl.conf ] && grep -q '^\[boot\]' /etc/wsl.conf; then
    echo "  Ya tienes una sección [boot] en /etc/wsl.conf sin systemd=true."
    echo "  Diff propuesto:"
    echo "    + systemd=true   (dentro de [boot])"
    ask_yn "  ¿Aplico?" || { echo "Cancelado."; exit 1; }
    sudo sed -i '/^\[boot\]/a systemd=true' /etc/wsl.conf
  else
    printf '\n[boot]\nsystemd=true\n' | sudo tee -a /etc/wsl.conf >/dev/null
  fi
  echo "✓ /etc/wsl.conf actualizado."
  echo ""
  echo "Ahora, desde PowerShell en Windows corre:"
  echo "    wsl --shutdown"
  echo "Reabre esta terminal y vuelve a correr ./install.sh"
  exit 0
}

CC_PLAT="$(cc_platform)"
echo "ComandOS -> instalando desde $REPO (plataforma: $CC_PLAT)"
mkdir -p "$BIN" "$HOOKS/dash" "$HOOKS/state" \
         "$HOME/.config/systemd/user" \
         "$HOME/.config/kitty" \
         "$HOME/.local/share/applications" \
         "$HOME/.local/share/icons/hicolor/scalable/apps"

# Binarios (symlink: los updates del repo se reflejan solos)
for f in "$REPO"/bin/*; do ln -sf "$f" "$BIN/$(basename "$f")"; done
chmod +x "$REPO"/bin/* "$REPO"/hooks/cc-notify.sh "$REPO"/hooks/cc-status.sh "$REPO"/hooks/cc-usage-tool.sh "$REPO"/adapters/grok-hooks.py

# Hooks + dashboard
for f in cc-notify.sh cc-status.sh cc-usage-tool.sh; do ln -sf "$REPO/hooks/$f" "$HOOKS/$f"; done
ln -sf "$REPO/adapters/grok-hooks.py" "$BIN/grok-hooks.py"
for f in index.html sw.js manifest.webmanifest icon-192.png icon-512.png term.html session-config.js session-controls.js workspace.js workspace-layout.js workspace-dock.js quick-terminal.js command-sidebar.js chain-builder.js device-drafts.js work-marks.js pomodoro.js ui-sounds.js notifications.js workspace.css analytics.js analytics-render.js analytics.css extensions.html extensions.js extensions.css buttons.css; do
  ln -sf "$REPO/dash/$f" "$HOOKS/dash/$f"
done
# Iconos Lucide y assets bundleados (xterm.js, fuentes): el terminal web
# remoto los carga via cc-dash en /icons y /assets.
ln -sfn "$REPO/dash/icons" "$HOOKS/dash/icons"
ln -sfn "$REPO/assets" "$HOOKS/dash/assets"
# N5/N4: lector de novedades, ajustes de push y dependencias vendorizadas
# (versión y sha256 en dash/vendor/vendored.json; nada se carga de un CDN).
for f in news-reader.js push-settings.js; do ln -sf "$REPO/dash/$f" "$HOOKS/dash/$f"; done
ln -sfn "$REPO/dash/vendor" "$HOOKS/dash/vendor"
[ -f "$HOOKS/cc-notify.conf" ] || cp "$REPO/hooks/cc-notify.conf.example" "$HOOKS/cc-notify.conf"
# Config de avisos: solo el dueno (0600)
chmod 600 "$HOOKS/cc-notify.conf" 2>/dev/null || true

# WSL instala jq de forma asistida; haz ese preflight antes de registrar. En
# las demas plataformas jq es un requisito externo y el helper avisa si falta.
if [ "$CC_PLAT" = "linux-wsl-ubuntu" ]; then
  cc_systemd_ok || _cc_wsl_enable_systemd
  _cc_wsl_install_deps
fi

# Hooks de Claude Code en ~/.claude/settings.json (los demás agentes van
# vía cc-agents setup). Corre antes del setup falible de UI/servicios.
cc_register_claude_hooks "$HOOKS/cc-notify.sh" "$HOOKS/cc-usage-tool.sh"

# Gateway Claude/Codex/Grok fijado por ComandOS. Se recompila solo si
# falta o el source vendorizado es más nuevo; no cambia el enabled-state.
GATEWAY_SRC="$REPO/vendor/claude-codex"
GATEWAY_BIN="$BIN/cc-model-proxy"
if [ -f "$GATEWAY_SRC/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
  if [ ! -x "$GATEWAY_BIN" ] || find "$GATEWAY_SRC/src" -type f -newer "$GATEWAY_BIN" -print -quit | grep -q .; then
    echo "  Compilando gateway Claude/Codex/Grok (release, locked)..."
    cargo build --release --locked --manifest-path "$GATEWAY_SRC/Cargo.toml"
    install -m 0755 "$GATEWAY_SRC/target/release/claude-codex" "$GATEWAY_BIN"
  fi
elif [ ! -x "$GATEWAY_BIN" ]; then
  echo "  (cargo no disponible: instala Rust para compilar cc-model-proxy)" >&2
fi


# Config de terminal
ln -sf "$REPO/config/kitty.conf" "$HOME/.config/kitty/kitty.conf"
ln -sf "$REPO/config/terminal-replies.conf" "$HOOKS/terminal-replies.conf"
if [ ! -e "$HOME/.tmux.conf" ]; then
  ln -sf "$REPO/config/tmux.conf" "$HOME/.tmux.conf"
else
  echo "  (ya tienes ~/.tmux.conf; revisa config/tmux.conf y mezcla a mano)"
fi
if ! grep -Fq 'source-file -q ~/.claude/hooks/terminal-replies.conf' "$HOME/.tmux.conf"; then
  printf '\n# ComandOS: respuestas de identificación de terminal\nsource-file -q ~/.claude/hooks/terminal-replies.conf\n' >> "$HOME/.tmux.conf"
fi

# App de escritorio (Exec/Icon apuntando a este usuario)
sed "s|__HOME__|$HOME|g" "$REPO/dash/comandos.desktop.in" \
  > "$HOME/.local/share/applications/comandos.desktop" 2>/dev/null || true
cp "$REPO/dash/comandos.svg" "$HOME/.local/share/icons/hicolor/scalable/apps/centro-claude.svg" 2>/dev/null || true

# Fuente JetBrainsMono Nerd Font (bundleada en assets/fonts/JetBrainsMono).
# Se instala en la carpeta de usuario y se refresca el caché de fuentes.
# Idempotente: cp -u solo copia si el archivo cambió.
_cc_install_fonts() {
  local src="$REPO/assets/fonts/JetBrainsMono" dest
  [ -d "$src" ] || return 0
  case "$CC_PLAT" in
    darwin)   dest="$HOME/Library/Fonts/ComandOS" ;;
    *)        dest="$HOME/.local/share/fonts/comandos" ;;
  esac
  mkdir -p "$dest"
  local changed=0
  for f in "$src"/*.ttf; do
    [ -f "$f" ] || continue
    if ! cmp -s "$f" "$dest/$(basename "$f")" 2>/dev/null; then
      cp "$f" "$dest/"; changed=1
    fi
  done
  # OFL/README junto a los TTF para atribución local (opcional, no bloqueante).
  cp -u "$src/OFL.txt" "$dest/OFL.txt" 2>/dev/null || true
  if [ "$changed" = "1" ] && command -v fc-cache >/dev/null 2>&1; then
    fc-cache -f "$dest" >/dev/null 2>&1 || true
  fi
}
_cc_install_fonts

# Servicios: dispatch por plataforma. macOS/Linux nativo mantienen el
# comportamiento anterior byte-a-byte; linux-wsl-ubuntu se rellena en Tasks 5–6.
case "$CC_PLAT" in
  darwin)
    mkdir -p "$HOME/Library/LaunchAgents"
    cat > "$HOME/Library/LaunchAgents/com.0xai.cc-dash.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.0xai.cc-dash</string>
  <key>ProgramArguments</key><array>
    <string>$BIN/cc-dash</string><string>--no-open</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <!-- launchd no hereda el PATH del shell: sin Homebrew aquí, cc-dash no
         encuentra tmux y /state (y toda acción tmux) muere con conexión vacía. -->
    <key>PATH</key><string>$BIN:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict></plist>
PLIST
    launchctl unload "$HOME/Library/LaunchAgents/com.0xai.cc-dash.plist" 2>/dev/null || true
    launchctl load "$HOME/Library/LaunchAgents/com.0xai.cc-dash.plist" 2>/dev/null || true
    echo "  macOS: cc-dash como LaunchAgent. App nativa (cc-app) ya disponible; requiere"
    echo "  'pip install pyobjc-framework-Cocoa pyobjc-framework-WebKit' y"
    echo "  'brew install tmux jq ttyd' (motor, hooks y pestanas)."
    ;;
  linux-wsl-ubuntu)
    # Dependencias y systemd ya se validaron antes de registrar hooks.
    # Configura los servicios de usuario (mismo flujo que linux-native).
    for s in cc-dash cc-notifyd cc-proxy; do
      ln -sf "$REPO/systemd/$s.service" "$HOME/.config/systemd/user/$s.service"
    done
    # Modelos más nuevos de Claude Code y Codex cada 6 h (cc-model-sync).
    ln -sf "$REPO/systemd/cc-model-sync.service" "$HOME/.config/systemd/user/cc-model-sync.service"
    ln -sf "$REPO/systemd/cc-model-sync.timer" "$HOME/.config/systemd/user/cc-model-sync.timer"
    systemctl --user daemon-reload
    systemctl --user enable --now cc-dash.service cc-notifyd.service 2>/dev/null || true
    systemctl --user enable --now cc-model-sync.timer 2>/dev/null || true
    [ "${COMANDOS_RETIRE_TELEGRAM:-0}" = "1" ] && cc_retire_telegram
    # Shortcut en el menú Inicio de Windows (native .lnk + .ico).
    "$BIN/cc-winstart" 2>&1 | sed 's/^/  /' || \
      echo "  (no pude publicar en Start Menu; correlo a mano: cc-winstart)"
    ;;
  linux-native|linux-other)
    [ "$CC_PLAT" = "linux-other" ] && echo "  (distro Linux no probada; sigo con el flujo Linux estándar)"
    for s in cc-dash cc-notifyd cc-proxy; do
      ln -sf "$REPO/systemd/$s.service" "$HOME/.config/systemd/user/$s.service"
    done
    # Modelos más nuevos de Claude Code y Codex cada 6 h (cc-model-sync).
    ln -sf "$REPO/systemd/cc-model-sync.service" "$HOME/.config/systemd/user/cc-model-sync.service"
    ln -sf "$REPO/systemd/cc-model-sync.timer" "$HOME/.config/systemd/user/cc-model-sync.timer"
    systemctl --user daemon-reload
    systemctl --user enable --now cc-dash.service cc-notifyd.service 2>/dev/null || true
    systemctl --user enable --now cc-model-sync.timer 2>/dev/null || true
    # Telegram ya no forma parte de ComandOS. La migracion que detiene la
    # unidad vieja se activa solo al instalar el candidato aprobado (R3).
    [ "${COMANDOS_RETIRE_TELEGRAM:-0}" = "1" ] && cc_retire_telegram
    ;;
esac

# Conectar otros agentes instalados (codex, opencode, gemini, agy)
"$BIN/cc-agents" setup || true

echo ""
echo "Listo. Abre el tablero en http://127.0.0.1:4777 o la app 'ComandOS'."
echo "Agentes conectados: corre 'cc-agents' para ver el estado."
echo "Dependencias sugeridas: tmux jq xclip wmctrl piper (voz) kitty (terminal)."
echo "Para el celular (seguro): tailscale + qrencode + ttyd, y corre cc-mobile."
echo "Terminal REAL en el celular: instala ttyd y corre cc-webterm (lo enruta cc-mobile)."

echo ""
echo "Diagnóstico:  cc-doctor      (o cc-doctor --fix para arreglos)"

if [ "$CC_PLAT" = "linux-wsl-ubuntu" ]; then
  echo ""
  echo "WSL: busca 'ComandOS' en el menú Inicio de Windows,"
  echo "     o abre Edge en http://127.0.0.1:4777"
fi
