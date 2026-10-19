#!/usr/bin/env python3
"""Private account discovery for subscription-backed agent CLIs.

Public helpers return aliases and display identity only. Paths and auth payloads
remain inside this module.
"""
from __future__ import annotations

import base64
import json
import os
import re
from pathlib import Path
from typing import Any

_ALIAS_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")


class AccountError(ValueError):
    pass


def _spec(registry: dict[str, Any], provider: str) -> dict[str, Any]:
    spec = (registry.get("harnesses") or {}).get(provider)
    if not isinstance(spec, dict) or not (spec.get("capabilities") or {}).get("accounts"):
        raise AccountError(f"{provider}: cuentas no soportadas")
    for key in ("defaultHome", "accountsRoot", "authFile", "accountEnv"):
        if not spec.get(key):
            raise AccountError(f"{provider}: registro de cuentas incompleto")
    return spec


def validate_alias(alias: str) -> str:
    alias = str(alias or "main")
    if alias != "main" and (not _ALIAS_RE.fullmatch(alias) or alias.startswith((".", "-"))):
        raise AccountError("alias de cuenta invalido")
    return alias


def account_home(registry: dict[str, Any], provider: str, alias: str = "main") -> Path:
    spec = _spec(registry, provider)
    alias = validate_alias(alias)
    if alias == "main":
        return Path(os.path.expanduser(spec["defaultHome"])).resolve()
    root = Path(os.path.expanduser(spec["accountsRoot"])).resolve()
    candidate = root / alias
    # resolve(strict=False) still resolves existing symlink parents. A named
    # account may not exist yet (login flow), but it may never escape its root.
    resolved = candidate.resolve(strict=False)
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        raise AccountError("cuenta fuera de accountsRoot") from exc
    if candidate.is_symlink():
        raise AccountError("aliases de cuenta no pueden ser symlinks")
    return resolved


def account_environment(registry: dict[str, Any], provider: str, alias: str) -> dict[str, str]:
    """Entorno para lanzar el CLI con una cuenta. «main» es la casa por defecto: SIN
    variable, igual que los procesos que el usuario abre a mano. Fijarla al mismo
    directorio no es inocuo: Claude Code mueve su ~/.claude.json (MCPs de usuario,
    confianza de carpetas) a $CLAUDE_CONFIG_DIR/.claude.json, otro archivo."""
    spec = _spec(registry, provider)
    if validate_alias(alias) == "main":
        return {}
    return {str(spec["accountEnv"]): str(account_home(registry, provider, alias))}


def _json(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def _jwt_email(token: Any) -> str:
    if not isinstance(token, str) or token.count(".") < 2:
        return ""
    try:
        part = token.split(".")[1]
        part += "=" * (-len(part) % 4)
        payload = json.loads(base64.urlsafe_b64decode(part))
        return str(payload.get("email") or "") if isinstance(payload, dict) else ""
    except Exception:
        return ""


def _claude_api_key(home: Path) -> bool:
    """Cuenta Claude por API key: sin OAuth, su settings.json declara apiKeyHelper
    o env.ANTHROPIC_API_KEY y el CLI la usa con CLAUDE_CONFIG_DIR apuntando a ella."""
    settings = _json(home / "settings.json")
    env = settings.get("env") if isinstance(settings.get("env"), dict) else {}
    return bool(settings.get("apiKeyHelper") or env.get("ANTHROPIC_API_KEY"))


def _auth(provider: str, home: Path, auth_file: str) -> tuple[bool, str, str]:
    data = _json(home / auth_file)
    identity = ""
    subscription = False
    if provider == "claude":
        oauth = data.get("claudeAiOauth") if isinstance(data.get("claudeAiOauth"), dict) else {}
        subscription = bool(oauth.get("accessToken") or oauth.get("refreshToken"))
        meta = _json(home / ".claude.json")
        if not meta:
            meta = _json(home.parent / ".claude.json")
        account = meta.get("oauthAccount") if isinstance(meta.get("oauthAccount"), dict) else {}
        identity = str(account.get("emailAddress") or "")
    elif provider == "codex":
        tokens = data.get("tokens") if isinstance(data.get("tokens"), dict) else {}
        subscription = bool(tokens.get("access_token") or tokens.get("refresh_token"))
        identity = _jwt_email(tokens.get("id_token"))
    elif provider == "grok":
        record = next((value for value in data.values() if isinstance(value, dict)), {})
        subscription = bool(record.get("refresh_token"))
        identity = str(record.get("email") or record.get("first_name") or record.get("user_id") or record.get("principal_id") or "")
    if subscription:
        return True, identity, "ready"
    if provider == "claude" and _claude_api_key(home):
        return True, identity or "API key", "ready"
    if data:
        return False, identity, "unsupported_auth"
    return False, "", "login_required"


# «<alias>.lock» es el candado que un proceso deja junto a la carpeta de la cuenta
# (p. ej. proper-lockfile de Claude Code); nunca es una cuenta.
def list_accounts(registry: dict[str, Any], provider: str) -> list[dict[str, Any]]:
    spec = _spec(registry, provider)
    aliases = ["main"]
    root = Path(os.path.expanduser(spec["accountsRoot"]))
    try:
        aliases.extend(sorted(
            item.name for item in root.iterdir()
            if item.is_dir() and not item.is_symlink() and _ALIAS_RE.fullmatch(item.name)
            and not item.name.startswith((".", "-")) and not item.name.endswith(".lock")
        ))
    except OSError:
        pass
    output = []
    for alias in aliases:
        try:
            home = account_home(registry, provider, alias)
            authenticated, identity, state = _auth(provider, home, str(spec["authFile"]))
        except AccountError:
            continue
        output.append({
            "provider": provider,
            "alias": alias,
            "identity": identity,
            "authenticated": authenticated,
            "state": state,
            "selectable": authenticated,
        })
    return output


def public_accounts(registry: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    output = {}
    for provider, spec in (registry.get("harnesses") or {}).items():
        if (spec.get("capabilities") or {}).get("accounts"):
            try:
                output[provider] = list_accounts(registry, provider)
            except AccountError:
                output[provider] = []
    return output


_LIMIT_ORDER = ("session", "weekly_all", "weekly_scoped", "window")
_LIMIT_SHORT = {"session": "5 h", "weekly_all": "semana", "window": "semana"}


def account_menu(accounts: list[dict[str, Any]], limits: list[dict[str, Any]], provider: str) -> list[dict[str, Any]]:
    """Cuentas de un CLI con su uso, para el botón «Cuenta» del pane (grill 2-oct):
    alias, identidad, si se puede elegir y sus límites (5 h, semana, semana por modelo)."""
    out = []
    for account in accounts or []:
        alias = str(account.get("alias") or "")
        if not _ALIAS_RE.fullmatch(alias):
            continue
        mine = [l for l in limits or [] if l.get("provider") == provider and l.get("account") == alias]
        mine.sort(key=lambda l: _LIMIT_ORDER.index(l.get("kind")) if l.get("kind") in _LIMIT_ORDER else len(_LIMIT_ORDER))
        rows = []
        for limit in mine:
            label = _LIMIT_SHORT.get(limit.get("kind")) or str(limit.get("label") or "").replace("Semana ", "").strip() or "límite"
            try:
                percent = max(0, min(100, round(float(limit.get("percent") or 0))))
            except (TypeError, ValueError):
                continue
            rows.append({"label": label, "percent": percent, "resetsAt": limit.get("resets_at")})
        out.append({"alias": alias, "identity": str(account.get("identity") or ""),
                    "selectable": bool(account.get("selectable")), "limits": rows})
    return out
