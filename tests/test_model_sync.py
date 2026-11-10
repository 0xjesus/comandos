import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "lib"))
import model_sync  # noqa: E402

EFFORTS = ["low", "medium", "high"]


def _registry():
    return {
        "version": 2,
        "motors": {
            "claude": {"label": "Claude", "models": [
                {"id": "claude-fable-5-1", "name": "Fable 5.1", "tag": "más inteligente · 1M ctx",
                 "efforts": EFFORTS + ["max"], "defaultEffort": "high"},
                {"id": "claude-opus-5-5", "name": "Opus 5.5", "tag": "potente",
                 "efforts": EFFORTS, "defaultEffort": "high"},
            ]},
            "codex": {"label": "Codex", "models": [
                {"id": "gpt-6.1-sol", "name": "GPT-6.1 Sol", "tag": "el más reciente",
                 "efforts": EFFORTS, "defaultEffort": "high"},
                {"id": "gpt-6-astra", "name": "GPT-6 Astra", "tag": "más inteligente",
                 "efforts": EFFORTS, "defaultEffort": "high"},
                {"id": "gpt-6-luna", "name": "GPT-6 Luna", "tag": "rápido",
                 "efforts": EFFORTS, "defaultEffort": "high"},
            ]},
        },
    }


DISCOVERED = {
    "claude": ["claude-fable-5", "claude-fable-5-1", "claude-fable-5-1[1m]", "claude-fable-5-2",
               "claude-mythos-5-2", "claude-opus-5-5[1m]", "claude-opus-4-1-20250805",
               "claude-haiku-5-5"],
    "codex": ["gpt-5.6-luna", "gpt-6-luna", "gpt-6.2-astra", "gpt-6.2-astra-fast"],
}


def test_keys():
    assert model_sync.claude_key("claude-fable-5-2") == ("fable", (5, 2))
    assert model_sync.claude_key("claude-opus-5") == ("opus", (5,))
    assert model_sync.claude_key("claude-opus-5-5[1m]") is None
    assert model_sync.claude_key("claude-opus-4-1-20250805") is None
    assert model_sync.codex_key("gpt-6.1-sol") == ("sol", (6, 1))
    assert model_sync.codex_key("gpt-5.3-codex-spark") == ("codex-spark", (5, 3))
    assert model_sync.codex_key("gpt-5.2") == ("", (5, 2))
    assert model_sync.codex_key("gpt-6-luna-fast") is None


def test_names():
    assert model_sync.claude_name("claude-fable-5-2") == "Fable 5.2"
    assert model_sync.codex_name("gpt-6.2-astra") == "GPT-6.2 Astra"
    assert model_sync.codex_name("gpt-5.2") == "GPT-5.2"


def test_plan_adds_only_newer_versions_of_curated_families():
    plan = model_sync.plan_registry(_registry(), DISCOVERED)
    # mythos no está curado, haiku tampoco en este registro, luna no es más nueva.
    assert plan == [
        ("claude", "claude-fable-5-2", "claude-fable-5-1"),
        ("codex", "gpt-6.2-astra", "gpt-6-astra"),
    ]


def test_apply_inserts_above_sibling_and_demotes_it():
    reg = _registry()
    out = model_sync.apply_registry(reg, model_sync.plan_registry(reg, DISCOVERED))
    claude = out["motors"]["claude"]["models"]
    assert [m["id"] for m in claude] == ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5"]
    assert claude[0] == {"id": "claude-fable-5-2", "name": "Fable 5.2",
                         "tag": "más inteligente · 1M ctx", "efforts": EFFORTS + ["max"],
                         "defaultEffort": "high"}
    assert claude[1]["tag"] == model_sync.PREVIOUS_TAG
    codex = [m["id"] for m in out["motors"]["codex"]["models"]]
    assert codex == ["gpt-6.1-sol", "gpt-6.2-astra", "gpt-6-astra", "gpt-6-luna"]
    # El original no se toca.
    assert reg["motors"]["claude"]["models"][0]["id"] == "claude-fable-5-1"


def test_plan_is_idempotent():
    reg = _registry()
    reg = model_sync.apply_registry(reg, model_sync.plan_registry(reg, DISCOVERED))
    assert model_sync.plan_registry(reg, DISCOVERED) == []


def test_bump_defaults_keep_family_and_suffix():
    top = {"fable": "claude-fable-5-2", "opus": "claude-opus-5-5"}
    assert model_sync.bump_claude_default("claude-fable-5-1[1m]", top) == "claude-fable-5-2[1m]"
    assert model_sync.bump_claude_default("claude-fable-5-1", top) == "claude-fable-5-2"
    assert model_sync.bump_claude_default("claude-opus-5-5", top) == "claude-opus-5-5"
    assert model_sync.bump_claude_default("opus", top) == "opus"
    assert model_sync.bump_claude_default("claude-sonnet-4-6", top) == "claude-sonnet-4-6"
    ctop = {"sol": "gpt-6.1-sol", "astra": "gpt-6-astra"}
    assert model_sync.bump_codex_default("gpt-6-sol", ctop) == "gpt-6.1-sol"
    assert model_sync.bump_codex_default("gpt-6-astra", ctop) == "gpt-6-astra"
    assert model_sync.bump_codex_default("gpt-5.6-terra", ctop) == "gpt-5.6-terra"


def test_bump_settings_text_preserves_formatting():
    text = '{\n  "env": {"model": "x"},\n  "model": "claude-fable-5-1[1m]",\n  "hooks": {}\n}\n'
    out = model_sync.bump_settings_text(text, {"fable": "claude-fable-5-2"})
    assert out == text.replace("claude-fable-5-1[1m]", "claude-fable-5-2[1m]")
    assert model_sync.bump_settings_text('{"model": "opus"}', {"fable": "claude-fable-5-2"}) is None
    assert model_sync.bump_settings_text("not json", {}) is None


def test_bump_codex_toml_only_top_level():
    text = 'model = "gpt-6-sol"\nmodel_reasoning_effort = "high"\n\n[profiles.x]\nmodel = "gpt-6-sol"\n'
    out = model_sync.bump_codex_toml_text(text, {"sol": "gpt-6.1-sol"})
    assert out == text.replace('model = "gpt-6-sol"\nmodel_reasoning', 'model = "gpt-6.1-sol"\nmodel_reasoning', 1)
    assert out.count('"gpt-6-sol"') == 1
    only_profile = '[profiles.x]\nmodel = "gpt-6-sol"\n'
    assert model_sync.bump_codex_toml_text(only_profile, {"sol": "gpt-6.1-sol"}) is None


def test_proxy_extras_and_config_dir():
    extras = model_sync.proxy_extras(_registry(), DISCOVERED)
    assert "claude-fable-5-2" in extras["anthropic"]
    assert "claude-mythos-5-2" not in extras["anthropic"]
    assert "claude-opus-5-5[1m]" not in extras["anthropic"]
    assert "gpt-6.2-astra" in extras["codex"] and "gpt-6.1-sol" in extras["codex"]
    assert "gpt-6.2-astra-fast" not in extras["codex"]
    assert model_sync.proxy_config_dir({"CCP_CONFIG_DIR": "/x"}) == Path("/x")
    assert model_sync.proxy_config_dir({"XDG_CONFIG_HOME": "/c"}) == Path("/c/claude-code-proxy")


def test_codex_update_command(monkeypatch):
    monkeypatch.setattr(model_sync, "_which", lambda n: f"/fake/bin/{n}")
    bun = model_sync.codex_update_command(
        "/h/.bun/install/global/node_modules/@openai/codex/bin/codex.js")
    assert bun == ["/fake/bin/bun", "add", "-g", "@openai/codex@latest"]
    npm = model_sync.codex_update_command(
        "/nonexistent/node/lib/node_modules/@openai/codex/bin/codex.js")
    assert npm == ["/fake/bin/npm", "i", "-g", "@openai/codex@latest"]
    assert model_sync.codex_update_command("/opt/codex") is None
    assert model_sync.codex_update_command(None) is None


def test_sync_end_to_end(tmp_path):
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    (home / ".claude-accounts" / "work").mkdir(parents=True)
    (home / ".codex").mkdir()
    settings = home / ".claude" / "settings.json"
    settings.write_text('{\n  "model": "claude-fable-5-1[1m]"\n}\n')
    (home / ".claude-accounts" / "work" / "settings.json").write_text('{"model": "opus"}\n')
    (home / ".codex" / "config.toml").write_text('model = "gpt-6-astra"\n')
    reg_path = tmp_path / "providers.json"
    reg_path.write_text(model_sync.dump_registry(_registry()))
    env = {"CCP_CONFIG_DIR": str(tmp_path / "proxy")}

    dry = model_sync.sync(dry_run=True, update=False, home=home, registry_path=reg_path,
                          discover=lambda: DISCOVERED, env=env, now=1)
    assert [a["id"] for a in dry["added"]] == ["claude-fable-5-2", "gpt-6.2-astra"]
    assert json.loads(reg_path.read_text()) == _registry()
    assert not (tmp_path / "proxy").exists()

    rep = model_sync.sync(update=False, home=home, registry_path=reg_path,
                          discover=lambda: DISCOVERED, env=env, now=1)
    assert json.loads(settings.read_text())["model"] == "claude-fable-5-2[1m]"
    assert (home / ".codex" / "config.toml").read_text() == 'model = "gpt-6.2-astra"\n'
    assert {d["to"] for d in rep["defaults"]} == {"claude-fable-5-2[1m]", "gpt-6.2-astra"}
    proxy = json.loads((tmp_path / "proxy" / "extra-models.json").read_text())
    assert "claude-fable-5-2" in proxy["anthropic"] and "gpt-6.2-astra" in proxy["codex"]
    assert model_sync.changed(rep)

    again = model_sync.sync(update=False, home=home, registry_path=reg_path,
                            discover=lambda: DISCOVERED, env=env, now=2)
    assert again["added"] == [] and again["defaults"] == []
    assert not again["proxy"]["changed"]
    assert not model_sync.changed(again)
    assert model_sync.summary(again) == ["modelos: sin novedades"]
