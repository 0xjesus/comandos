"""Mantiene ComandOS al día con los modelos más nuevos de Claude Code y Codex.

`model_watch` solo avisa ("hay modelos nuevos"). Este módulo aplica:

1. Actualiza los CLIs: `claude update` y `@openai/codex@latest` con el mismo
   gestor con el que está instalado (bun o npm).
2. Descubre modelos con `model_watch.discover_models()` (binario de Claude,
   catálogo `models_cache.json` de Codex).
3. Por familia (Claude: fable/opus/sonnet/haiku) o variante (Codex: sol, luna,
   terra, astra…), si aparece una versión más nueva que la más nueva del
   registro, la agrega a `config/providers.json` arriba de su hermana, con sus
   esfuerzos y su etiqueta; la hermana pasa a "generación anterior". Familias
   que el registro no cura (p. ej. mythos, de acceso restringido) no entran.
4. Sube los modelos por defecto a la versión nueva de la MISMA familia:
   `~/.claude/settings.json` (y cuentas en `~/.claude-accounts/*`) conservando
   sufijos como `[1m]`, y el `model` de nivel superior de `~/.codex/config.toml`.
   Los alias (`opus`, `fable`) no se tocan: ya siguen solos al más nuevo.
5. Escribe `extra-models.json` del gateway (`cc-model-proxy`), que lo relee en
   caliente: los modelos nuevos se listan y enrutan sin reiniciarlo.

Todo cambio de archivo es atómico (`.tmp` + `os.replace`) y solo ocurre si el
contenido cambia. `dry_run=True` no escribe ni actualiza nada.
"""
from __future__ import annotations

import copy
import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path

import model_watch
from providers import DEFAULT_PATH as REGISTRY_PATH
from providers import which as _which

CLAUDE_FAMILIES = ("fable", "opus", "sonnet", "haiku")
PREVIOUS_TAG = "generación anterior"
CODEX_PACKAGE = "@openai/codex"
PROXY_FILE = "extra-models.json"
CLI_TIMEOUT = 600

_CLAUDE_ID = re.compile(r"claude-([a-z]+)-(\d+(?:-\d+)?)$")
_CLAUDE_DEFAULT = re.compile(r"(claude-([a-z]+)-(\d+(?:-\d+)?))((?:\[[^\]]*\])?)$")
_CODEX_ID = re.compile(r"gpt-(\d+(?:\.\d+)?)(?:-([a-z][a-z0-9-]*))?$")
_TOML_MODEL = re.compile(r'^(\s*model\s*=\s*")([^"]+)(".*)$')


# --------------------------------------------------------------------- claves

def claude_key(mid):
    """('fable', (5, 2)) para `claude-fable-5-2`; None si no es un id canónico
    (con fecha, con corchetes o de otra forma)."""
    m = _CLAUDE_ID.fullmatch(mid or "")
    if not m:
        return None
    return m.group(1), tuple(int(x) for x in m.group(2).split("-"))


def codex_key(mid):
    """('sol', (6, 1)) para `gpt-6.1-sol`; ('', (5, 2)) para `gpt-5.2`."""
    m = _CODEX_ID.fullmatch(mid or "")
    if not m or (m.group(2) or "").endswith("fast"):
        return None
    return m.group(2) or "", tuple(int(x) for x in m.group(1).split("."))


def newest(ids, key, allow=None):
    """{familia: id} con la versión más alta de cada familia."""
    best = {}
    for mid in ids or ():
        k = key(mid) if isinstance(mid, str) else None
        if not k or (allow is not None and k[0] not in allow):
            continue
        fam, ver = k
        if fam not in best or ver > best[fam][0]:
            best[fam] = (ver, mid)
    return {fam: mid for fam, (_ver, mid) in best.items()}


def claude_name(mid):
    fam, ver = claude_key(mid)
    return f"{fam.capitalize()} {'.'.join(map(str, ver))}"


def codex_name(mid):
    variant, ver = codex_key(mid)
    base = f"GPT-{'.'.join(map(str, ver))}"
    return f"{base} {variant.replace('-', ' ').title()}" if variant else base


MOTORS = {
    "claude": {"key": claude_key, "name": claude_name, "allow": CLAUDE_FAMILIES},
    "codex": {"key": codex_key, "name": codex_name, "allow": None},
}


# ------------------------------------------------------------------- registro

def registry_ids(reg, motor):
    spec = ((reg or {}).get("motors") or {}).get(motor) or {}
    return [m.get("id") for m in spec.get("models") or [] if isinstance(m, dict)]


def plan_registry(reg, discovered):
    """Modelos a agregar: [(motor, id_nuevo, id_hermana)]. Solo familias que
    el registro ya cura y solo versiones más nuevas que su máximo."""
    plan = []
    for motor, rules in MOTORS.items():
        have = registry_ids(reg, motor)
        if not have:
            continue
        top = newest(have, rules["key"], rules["allow"])
        found = newest(discovered.get(motor) or [], rules["key"], rules["allow"])
        for fam, mid in sorted(found.items()):
            sibling = top.get(fam)
            if not sibling or mid in have:
                continue
            if rules["key"](mid)[1] > rules["key"](sibling)[1]:
                plan.append((motor, mid, sibling))
    return plan


def apply_registry(reg, plan):
    """Copia de `reg` con los modelos del plan insertados sobre su hermana."""
    reg = copy.deepcopy(reg)
    for motor, mid, sibling in plan:
        models = reg["motors"][motor]["models"]
        idx = next(i for i, m in enumerate(models) if m.get("id") == sibling)
        entry = copy.deepcopy(models[idx])
        entry["id"] = mid
        entry["name"] = MOTORS[motor]["name"](mid)
        models[idx]["tag"] = PREVIOUS_TAG
        models.insert(idx, entry)
    return reg


# ------------------------------------------------------------ modelos default

def bump_claude_default(value, top):
    """`claude-fable-5-1[1m]` -> `claude-fable-5-2[1m]` si hay Fable más nuevo."""
    m = _CLAUDE_DEFAULT.fullmatch(value or "")
    if not m:
        return value
    fam, suffix = m.group(2), m.group(4)
    cur = claude_key(m.group(1))
    new = top.get(fam)
    if cur and new and claude_key(new)[1] > cur[1]:
        return new + suffix
    return value


def bump_codex_default(value, top):
    """`gpt-6-astra` -> `gpt-6.1-astra` solo si existe la misma variante."""
    cur = codex_key(value or "")
    if not cur:
        return value
    new = top.get(cur[0])
    if new and codex_key(new)[1] > cur[1]:
        return new
    return value


def bump_settings_text(text, top):
    """Nuevo texto de un settings.json de Claude con el `model` subido, o
    None. Reemplazo textual para no reformatear el archivo del usuario."""
    try:
        data = json.loads(text)
    except ValueError:
        return None
    old = data.get("model") if isinstance(data, dict) else None
    if not isinstance(old, str):
        return None
    new = bump_claude_default(old, top)
    if new == old:
        return None
    pattern = re.compile(r'("model"\s*:\s*")' + re.escape(old) + '"')
    if len(pattern.findall(text)) != 1:
        return None
    out = pattern.sub(lambda m: m.group(1) + new + '"', text)
    if json.loads(out).get("model") != new:
        return None
    return out


def bump_codex_toml_text(text, top):
    """Nuevo texto de config.toml con el `model` de nivel superior subido."""
    lines = text.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.lstrip().startswith("["):
            return None
        m = _TOML_MODEL.match(line.rstrip("\n"))
        if m:
            new = bump_codex_default(m.group(2), top)
            if new == m.group(2):
                return None
            end = "\n" if line.endswith("\n") else ""
            lines[i] = m.group(1) + new + m.group(3) + end
            return "".join(lines)
    return None


# -------------------------------------------------------------------- gateway

def proxy_config_dir(env=None):
    env = os.environ if env is None else env
    if env.get("CCP_CONFIG_DIR"):
        return Path(env["CCP_CONFIG_DIR"])
    base = env.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return Path(base) / "claude-code-proxy"


def proxy_extras(reg, discovered):
    """Ids que el gateway debe listar/aceptar además de los compilados."""
    claude = set(registry_ids(reg, "claude")) | set(
        newest(discovered.get("claude") or [], claude_key, CLAUDE_FAMILIES).values())
    codex = set(registry_ids(reg, "codex")) | set(discovered.get("codex") or [])
    return {
        "anthropic": sorted(m for m in claude if claude_key(m)),
        "codex": sorted(m for m in codex if codex_key(m)),
    }


# --------------------------------------------------------------------- disco

def write_if_changed(path, text, dry_run=False):
    path = Path(path)
    try:
        if path.read_text() == text:
            return False
    except OSError:
        pass
    if dry_run:
        return True
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    tmp.write_text(text)
    try:
        os.chmod(tmp, path.stat().st_mode & 0o777)
    except OSError:
        pass
    os.replace(tmp, path)
    return True


def dump_registry(reg):
    return json.dumps(reg, indent=2, ensure_ascii=False) + "\n"


# ---------------------------------------------------------------------- CLIs

def _run(cmd, timeout=CLI_TIMEOUT):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           stdin=subprocess.DEVNULL)
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except (OSError, subprocess.SubprocessError) as exc:
        return None, str(exc)


def _version(exe):
    code, out = _run([exe, "--version"], timeout=30)
    m = re.search(r"\d+\.\d+\.\d+", out or "") if code == 0 else None
    return m.group(0) if m else None


def codex_install_root(home=None):
    """Ruta real del paquete de Codex (detrás del wrapper codex-yolo)."""
    home = Path(home or os.path.expanduser("~"))
    manifest = home / ".local/share/comandos/codex-yolo/launcher.json"
    try:
        original = json.loads(manifest.read_text()).get("original")
        if original:
            return os.path.realpath(original)
    except (OSError, ValueError, AttributeError):
        pass
    exe = _which("codex")
    return os.path.realpath(exe) if exe else None


def codex_update_command(root):
    if not root:
        return None
    if "/.bun/install/global/" in root:
        bun = _which("bun")
        return [bun, "add", "-g", f"{CODEX_PACKAGE}@latest"] if bun else None
    if "/lib/node_modules/" in root:
        npm = os.path.join(root.split("/lib/node_modules/")[0], "bin", "npm")
        npm = npm if os.path.exists(npm) else _which("npm")
        return [npm, "i", "-g", f"{CODEX_PACKAGE}@latest"] if npm else None
    return None


def update_clis(run=_run):
    out = {}
    claude = _which("claude")
    if claude:
        before = _version(claude)
        code, text = run([claude, "update"])
        out["claude"] = {"before": before, "after": _version(claude), "ok": code == 0,
                         "detail": text.strip().splitlines()[-1:] if text else []}
    codex = _which("codex")
    cmd = codex_update_command(codex_install_root())
    if codex and cmd:
        before = _version(codex)
        code, text = run(cmd)
        after = _version(codex)
        out["codex"] = {"before": before, "after": after, "ok": code == 0 and after is not None,
                        "detail": text.strip().splitlines()[-1:] if text else []}
    return out


# --------------------------------------------------------------------- todo

def claude_settings_paths(home):
    home = Path(home)
    paths = [home / ".claude" / "settings.json"]
    paths += sorted((home / ".claude-accounts").glob("*/settings.json"))
    return [p for p in paths if p.is_file()]


def sync(*, dry_run=False, update=True, home=None, registry_path=None,
         discover=None, env=None, now=None):
    home = Path(home or os.path.expanduser("~"))
    registry_path = Path(registry_path or REGISTRY_PATH)
    report = {"checkedAt": int(now if now is not None else time.time()),
              "dryRun": dry_run, "cli": {}, "added": [], "defaults": [], "proxy": {}}

    if update and not dry_run:
        report["cli"] = update_clis()

    discovered = (discover or model_watch.discover_models)()
    report["discovered"] = {k: discovered.get(k) or [] for k in MOTORS}

    reg = json.loads(registry_path.read_text())
    plan = plan_registry(reg, discovered)
    if plan:
        reg = apply_registry(reg, plan)
        write_if_changed(registry_path, dump_registry(reg), dry_run)
        report["added"] = [{"motor": m, "id": mid, "replaces": sib} for m, mid, sib in plan]

    claude_top = newest(registry_ids(reg, "claude"), claude_key, CLAUDE_FAMILIES)
    codex_top = newest(registry_ids(reg, "codex"), codex_key)
    for path in claude_settings_paths(home):
        text = path.read_text()
        new = bump_settings_text(text, claude_top)
        if new:
            write_if_changed(path, new, dry_run)
            report["defaults"].append({"file": str(path), "from": json.loads(text)["model"],
                                       "to": json.loads(new)["model"]})
    codex_toml = Path(os.environ.get("CODEX_HOME") or home / ".codex") / "config.toml"
    if codex_toml.is_file():
        text = codex_toml.read_text()
        new = bump_codex_toml_text(text, codex_top)
        if new:
            write_if_changed(codex_toml, new, dry_run)
            old_m = next(_TOML_MODEL.match(l).group(2) for l in text.splitlines() if _TOML_MODEL.match(l))
            new_m = next(_TOML_MODEL.match(l).group(2) for l in new.splitlines() if _TOML_MODEL.match(l))
            report["defaults"].append({"file": str(codex_toml), "from": old_m, "to": new_m})

    extras = proxy_extras(reg, discovered)
    proxy_path = proxy_config_dir(env) / PROXY_FILE
    changed = write_if_changed(proxy_path, json.dumps(extras, indent=1) + "\n", dry_run)
    report["proxy"] = {"file": str(proxy_path), "changed": changed, "models": extras}
    return report


def summary(report):
    lines = []
    for cli, info in (report.get("cli") or {}).items():
        state = "ok" if info.get("ok") else "FALLÓ"
        if info.get("before") != info.get("after"):
            lines.append(f"{cli}: {info.get('before')} -> {info.get('after')} ({state})")
        else:
            lines.append(f"{cli}: {info.get('after')} al día ({state})")
    for a in report.get("added") or []:
        lines.append(f"registro: + {a['id']} (antes {a['replaces']})")
    for d in report.get("defaults") or []:
        lines.append(f"default: {d['from']} -> {d['to']} en {d['file']}")
    proxy = report.get("proxy") or {}
    if proxy.get("changed"):
        n = sum(len(v) for v in (proxy.get("models") or {}).values())
        lines.append(f"gateway: {n} modelos en {proxy['file']}")
    if not (report.get("added") or report.get("defaults") or proxy.get("changed")):
        lines.append("modelos: sin novedades")
    if report.get("dryRun"):
        lines.insert(0, "(simulación: no se escribió nada)")
    return lines


def changed(report):
    cli_moved = any(i.get("before") != i.get("after") for i in (report.get("cli") or {}).values())
    return bool(cli_moved or report.get("added") or report.get("defaults")
                or (report.get("proxy") or {}).get("changed"))


def notify(lines):
    exe = shutil.which("notify-send")
    if exe:
        _run([exe, "-a", "ComandOS", "Modelos actualizados", "\n".join(lines)], timeout=10)
