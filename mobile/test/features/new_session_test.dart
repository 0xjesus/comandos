import 'dart:convert';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/models/provider_registry.dart';
import 'package:comandos_mobile/features/new_session/new_session.dart';
import 'package:comandos_mobile/features/new_session/provider_mark.dart';
import 'package:comandos_mobile/features/new_session/wizard_model.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

// `GET /providers` of the live dashboard (21-nov-2026), trimmed to the fields
// the wizard reads; account identities and aliases anonymised.
const String providersJson =
    r'''{"version":2,"matrixHarnesses":["claude","codex","grok","acp","opencode","agy"],"harnesses":{"claude'''
    r'''":{"label":"Claude Code","icon":"anthropic","capabilities":{"accounts":true},"accounts":[{"provider"'''
    r''':"claude","alias":"main","identity":"person@example.com","authenticated":true,"state":"ready","selec'''
    r'''table":true,"motorSelectable":true},{"provider":"claude","alias":"work","identity":"","authenticated'''
    r'''":false,"state":"login_required","selectable":false,"motorSelectable":false},{"provider":"claude","a'''
    r'''lias":"side","identity":"person@example.com","authenticated":true,"state":"ready","selectable":true,'''
    r'''"motorSelectable":false},{"provider":"claude","alias":"extra","identity":"person@example.com","authe'''
    r'''nticated":true,"state":"ready","selectable":true,"motorSelectable":false}]},"codex":{"label":"Codex '''
    r'''CLI","icon":"openai","capabilities":{"accounts":true},"accounts":[{"provider":"codex","alias":"main"'''
    r''',"identity":"person@example.com","authenticated":true,"state":"ready","selectable":true,"motorSelect'''
    r'''able":true}]},"grok":{"label":"Grok Build","icon":"xai","capabilities":{"accounts":true},"accounts":'''
    r'''[{"provider":"grok","alias":"main","identity":"person@example.com","authenticated":true,"state":"rea'''
    r'''dy","selectable":true,"motorSelectable":true}]},"acp":{"label":"ComandOS ACP","icon":"layers","capab'''
    r'''ilities":{"accounts":false}},"opencode":{"label":"OpenCode","icon":"opencode","capabilities":{"accou'''
    r'''nts":false}},"agy":{"label":"Antigravity","icon":"gemini","capabilities":{"accounts":false}},"gemini'''
    r'''":{"label":"Gemini CLI","icon":"gemini","capabilities":{"accounts":false}},"shell":{"label":"Shell",'''
    r'''"icon":"terminal","capabilities":{"accounts":false}}},"motors":{"claude":{"label":"Claude","icon":"a'''
    r'''nthropic","models":[{"id":"claude-fable-5-2","name":"Fable 5.2","efforts":["low","medium","high","xh'''
    r'''igh","max"],"defaultEffort":"high"},{"id":"claude-fable-5-1","name":"Fable 5.1","efforts":["low","me'''
    r'''dium","high","xhigh","max"],"defaultEffort":"high"},{"id":"claude-opus-5-5","name":"Opus 5.5","effor'''
    r'''ts":["low","medium","high","xhigh","max"],"defaultEffort":"high"},{"id":"claude-sonnet-5-5","name":"'''
    r'''Sonnet 5.5","efforts":["low","medium","high","xhigh","max"],"defaultEffort":"high"},{"id":"claude-ha'''
    r'''iku-5-5","name":"Haiku 5.5","efforts":["low","medium","high","xhigh","max"],"defaultEffort":"high"}]'''
    r'''},"codex":{"label":"Codex","icon":"openai","models":[{"id":"gpt-6.1-sol","name":"GPT-6.1 Sol","effor'''
    r'''ts":["low","medium","high","xhigh","max","ultra"],"defaultEffort":"high"},{"id":"gpt-6-astra","name"'''
    r''':"GPT-6 Astra","efforts":["low","medium","high","xhigh","max","ultra"],"defaultEffort":"high"},{"id"'''
    r''':"gpt-6-sol","name":"GPT-6 Sol","efforts":["low","medium","high","xhigh","max","ultra"],"defaultEffo'''
    r'''rt":"high"},{"id":"gpt-6-luna","name":"GPT-6 Luna","efforts":["low","medium","high","xhigh","max"],"'''
    r'''defaultEffort":"medium"},{"id":"gpt-5.6-sol","name":"GPT-5.6 Sol","efforts":["low","medium","high","'''
    r'''xhigh","max","ultra"],"defaultEffort":"high"},{"id":"gpt-5.6-terra","name":"GPT-5.6 Terra","efforts"'''
    r''':["low","medium","high","xhigh","max","ultra"],"defaultEffort":"medium"},{"id":"gpt-5.6-luna","name"'''
    r''':"GPT-5.6 Luna","efforts":["low","medium","high","xhigh","max"],"defaultEffort":"medium"}]},"grok":{'''
    r'''"label":"Grok","icon":"xai","models":[{"id":"grok-4.7","name":"Grok 4.7","efforts":["xhigh","high","'''
    r'''medium","low"],"defaultEffort":"high"}]},"opencode":{"label":"OpenCode","icon":"opencode","models":['''
    r'''{"id":"opencode/big-pickle","name":"Big Pickle (free)","efforts":[],"defaultEffort":""},{"id":"openc'''
    r'''ode/nemotron-3-ultra-free","name":"Nemotron 3 Ultra (free)","efforts":[],"defaultEffort":""},{"id":"'''
    r'''openrouter/anthropic/claude-fable-5.1","name":"Fable 5.1 (OpenRouter)","efforts":[],"defaultEffort":'''
    r'''""},{"id":"openrouter/~openai/gpt-latest","name":"GPT latest (OpenRouter)","efforts":[],"defaultEffo'''
    r'''rt":""},{"id":"groq/llama-3.1-8b-instant","name":"Llama 3.1 8B (Groq)","efforts":[],"defaultEffort":'''
    r'''""},{"id":"groq/llama-3.3-70b-versatile","name":"Llama 3.3 70B (Groq)","efforts":[],"defaultEffort":'''
    r'''""},{"id":"groq/openai/gpt-oss-120b","name":"GPT-OSS 120B (Groq)","efforts":[],"defaultEffort":""},{'''
    r'''"id":"groq/openai/gpt-oss-20b","name":"GPT-OSS 20B (Groq)","efforts":[],"defaultEffort":""},{"id":"g'''
    r'''roq/qwen/qwen3.6-27b","name":"Qwen3.6 27B (Groq)","efforts":[],"defaultEffort":""},{"id":"groq/qwen/'''
    r'''qwen3.8-27b","name":"Qwen3.8 27B (Groq)","efforts":[],"defaultEffort":""}]},"agy":{"label":"Antigrav'''
    r'''ity","icon":"gemini","models":[{"id":"gemini-3.8-flash-high","name":"Gemini 3.8 Flash (high)","effor'''
    r'''ts":["low","medium","high"],"defaultEffort":"high"},{"id":"gemini-3.8-flash-medium","name":"Gemini 3'''
    r'''.8 Flash (medium)","efforts":["low","medium","high"],"defaultEffort":"high"},{"id":"gemini-3.8-flash'''
    r'''-low","name":"Gemini 3.8 Flash (low)","efforts":["low","medium","high"],"defaultEffort":"high"},{"id'''
    r'''":"claude-opus-5-5-high","name":"Claude Opus 5.5 (high)","efforts":["low","medium","high"],"defaultE'''
    r'''ffort":"high"},{"id":"claude-opus-5-5-medium","name":"Claude Opus 5.5 (medium)","efforts":["low","me'''
    r'''dium","high"],"defaultEffort":"high"},{"id":"claude-opus-5-5-low","name":"Claude Opus 5.5 (low)","ef'''
    r'''forts":["low","medium","high"],"defaultEffort":"high"},{"id":"claude-sonnet-5-5-high","name":"Claude'''
    r''' Sonnet 5.5 (high)","efforts":["low","medium","high"],"defaultEffort":"high"},{"id":"claude-sonnet-5'''
    r'''-5-medium","name":"Claude Sonnet 5.5 (medium)","efforts":["low","medium","high"],"defaultEffort":"hi'''
    r'''gh"},{"id":"claude-sonnet-5-5-low","name":"Claude Sonnet 5.5 (low)","efforts":["low","medium","high"'''
    r'''],"defaultEffort":"high"},{"id":"gpt-oss-120b-medium","name":"GPT-OSS 120B","efforts":["low","medium'''
    r'''","high"],"defaultEffort":"medium"}]}},"matrix":[{"id":"claude:claude","harness":"claude","motor":"c'''
    r'''laude","selectable":true,"productState":"supported","driver":"claude","accountScopes":["harness"],"r'''
    r'''eason":null},{"id":"claude:codex","harness":"claude","motor":"codex","selectable":true,"productState'''
    r'''":"supported","driver":"claude","accountScopes":["harness","motor"],"reason":null},{"id":"claude:gro'''
    r'''k","harness":"claude","motor":"grok","selectable":true,"productState":"supported","driver":"claude",'''
    r'''"accountScopes":["harness","motor"],"reason":null},{"id":"claude:opencode","harness":"claude","motor'''
    r'''":"opencode","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":'''
    r'''"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"claude:agy","harness":"clau'''
    r'''de","motor":"agy","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","mess'''
    r'''age":"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"codex:claude","harness'''
    r'''":"codex","motor":"claude","selectable":false,"productState":"unsupported_protocol","reason":{"code"'''
    r''':"unsupported_protocol","message":"Codex CLI solo admite Responses; Claude subscription no expone es'''
    r'''e protocolo."}},{"id":"codex:codex","harness":"codex","motor":"codex","selectable":true,"productStat'''
    r'''e":"supported","driver":"codex","accountScopes":["harness"],"reason":null},{"id":"codex:grok","harne'''
    r'''ss":"codex","motor":"grok","selectable":false,"productState":"supported","driver":"codex","accountSc'''
    r'''opes":["harness","motor"],"reason":{"code":"route_not_live_verified","message":"puente pendiente de '''
    r'''verificación E2E"}},{"id":"codex:opencode","harness":"codex","motor":"opencode","selectable":false,"'''
    r'''productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI no hospeda ese motor; u'''
    r'''sa el harness ACP de ComandOS"}},{"id":"codex:agy","harness":"codex","motor":"agy","selectable":fals'''
    r'''e,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI no hospeda ese motor'''
    r'''; usa el harness ACP de ComandOS"}},{"id":"grok:claude","harness":"grok","motor":"claude","selectabl'''
    r'''e":false,"productState":"subscription_incompatible","reason":{"code":"subscription_not_accepted","me'''
    r'''ssage":"Grok Build requiere API key Anthropic; Claude Max no se puede reenviar."}},{"id":"grok:codex'''
    r'''","harness":"grok","motor":"codex","selectable":false,"productState":"supported","driver":"grok","ac'''
    r'''countScopes":["harness","motor"],"reason":{"code":"route_not_live_verified","message":"puente pendie'''
    r'''nte de verificación E2E"}},{"id":"grok:grok","harness":"grok","motor":"grok","selectable":true,"prod'''
    r'''uctState":"supported","driver":"grok","accountScopes":["harness"],"reason":null},{"id":"grok:opencod'''
    r'''e","harness":"grok","motor":"opencode","selectable":false,"productState":"not_routed","reason":{"cod'''
    r'''e":"not_routed","message":"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"g'''
    r'''rok:agy","harness":"grok","motor":"agy","selectable":false,"productState":"not_routed","reason":{"co'''
    r'''de":"not_routed","message":"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"'''
    r'''acp:claude","harness":"acp","motor":"claude","selectable":true,"productState":"supported","driver":"'''
    r'''acp","accountScopes":["motor"],"reason":null},{"id":"acp:codex","harness":"acp","motor":"codex","sel'''
    r'''ectable":true,"productState":"supported","driver":"acp","accountScopes":["motor"],"reason":null},{"i'''
    r'''d":"acp:grok","harness":"acp","motor":"grok","selectable":true,"productState":"supported","driver":"'''
    r'''acp","accountScopes":["motor"],"reason":null},{"id":"acp:opencode","harness":"acp","motor":"opencode'''
    r'''","selectable":true,"productState":"supported","driver":"acp","accountScopes":[],"reason":null},{"id'''
    r'''":"acp:agy","harness":"acp","motor":"agy","selectable":true,"productState":"supported","driver":"acp'''
    r'''","accountScopes":[],"reason":null},{"id":"opencode:claude","harness":"opencode","motor":"claude","s'''
    r'''electable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI no ho'''
    r'''speda ese motor; usa el harness ACP de ComandOS"}},{"id":"opencode:codex","harness":"opencode","moto'''
    r'''r":"codex","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"e'''
    r'''sta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"opencode:grok","harness":"ope'''
    r'''ncode","motor":"grok","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","'''
    r'''message":"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"opencode:opencode"'''
    r''',"harness":"opencode","motor":"opencode","selectable":true,"productState":"supported","driver":"open'''
    r'''code","accountScopes":[],"reason":null},{"id":"opencode:agy","harness":"opencode","motor":"agy","sel'''
    r'''ectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI no hosp'''
    r'''eda ese motor; usa el harness ACP de ComandOS"}},{"id":"agy:claude","harness":"agy","motor":"claude"'''
    r''',"selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI no'''
    r''' hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"agy:codex","harness":"agy","motor":"cod'''
    r'''ex","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TUI'''
    r''' no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"agy:grok","harness":"agy","motor":"g'''
    r'''rok","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message":"esta TU'''
    r'''I no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"agy:opencode","harness":"agy","moto'''
    r'''r":"opencode","selectable":false,"productState":"not_routed","reason":{"code":"not_routed","message"'''
    r''':"esta TUI no hospeda ese motor; usa el harness ACP de ComandOS"}},{"id":"agy:agy","harness":"agy","'''
    r'''motor":"agy","selectable":true,"productState":"supported","driver":"agy","accountScopes":[],"reason"'''
    r''':null}]}''';

// `GET /fs/dirs?path=…` answers of the live dashboard.
const Map<String, Object?> portalDirs = <String, Object?>{
  'path': '/srv/dev/code/acme/portal',
  'exists': true,
  'base': '/srv/dev/code/acme/portal',
  'parent': '/srv/dev/code/acme',
  'missing': '',
  'home': '/srv/dev',
  'dirs': <Object?>[
    <String, Object?>{'name': 'apps', 'path': '/srv/dev/code/acme/portal/apps'},
    <String, Object?>{'name': 'docs', 'path': '/srv/dev/code/acme/portal/docs'},
  ],
};
const Map<String, Object?> missingDirs = <String, Object?>{
  'path': '/srv/dev/nonexist/xyz',
  'exists': false,
  'base': '/srv/dev',
  'parent': '',
  'missing': 'nonexist/xyz',
  'home': '/srv/dev',
  'dirs': <Object?>[],
};

ProviderRegistry registry() => ProviderRegistry.tryParse(jsonDecode(providersJson))!;

WizardModel freshModel() {
  final WizardModel m = WizardModel(registry(), WizardSelection.defaults());
  m.selectDefaults();
  m.normalize();
  return m;
}

http.Response jsonResponse(Object body, [int status = 200]) => http.Response.bytes(
      utf8.encode(jsonEncode(body)),
      status,
      headers: <String, String>{'content-type': 'application/json; charset=utf-8'},
    );

void main() {
  group('WizardModel (real /providers)', () {
    test('defaults are the web NS defaults: Codex CLI → Codex · gpt-6.1-sol · high', () {
      final WizardModel m = freshModel();
      expect(m.sel.routeId, 'codex:codex');
      expect(m.sel.model, 'gpt-6.1-sol');
      expect(m.sel.effort, 'high');
      expect(m.sel.harnessAccount, 'main');
      expect(m.sel.motorAccount, 'main');
      expect(m.note, 'Codex CLI → Codex · gpt-6.1-sol · high');
      expect(m.canCreate, isTrue);
      expect(m.createTitle, 'Crear la sesión con este setup');
    });

    test('harness row: matrixHarnesses order, labels, then Shell', () {
      final WizardModel m = freshModel();
      expect(m.harnessChips.map((WizardChip c) => c.label).toList(), <String>[
        'Claude Code', 'Codex CLI', 'Grok Build', 'ComandOS ACP', 'OpenCode', 'Antigravity', 'Shell',
      ]);
      expect(m.harnessChips.last.icon, 'terminal');
    });

    test('provider marks: glyphs for Anthropic/OpenAI/xAI/Gemini, mono badges otherwise', () {
      ProviderMarks.reset();
      expect(ProviderMarks.resolve('anthropic', ''), isNotNull);
      expect(ProviderMarks.resolve('openai', ''), isNotNull);
      expect(ProviderMarks.resolve('xai', ''), ProviderMarks.xai);
      expect(ProviderMarks.resolve('gemini', ''), isNotNull);
      expect(ProviderMarks.resolve('codex', 'gpt-6.1-sol'), isNotNull);
      for (final (String icon, String mono) in <(String, String)>[('layers', 'LA'), ('opencode', 'OP'), ('terminal', 'TE')]) {
        expect(ProviderMarks.resolve(icon, ''), isNull, reason: icon);
        expect(ProviderMarks.mono(icon, ''), mono);
      }
      // Antigravity's Claude models show the Anthropic «A».
      expect(ProviderMarks.resolve('agy', 'claude-opus-5-5-high'), isNotNull);
    });

    test('Codex CLI motors: Claude and Grok shown disabled with the reason, not_routed hidden', () {
      final WizardModel m = freshModel();
      final List<WizardChip> motors = m.motorChips;
      expect(motors.map((WizardChip c) => c.id).toList(), <String>['claude', 'codex', 'grok']);
      expect(motors[0].disabled, isTrue);
      expect(motors[0].title, contains('Responses'));
      expect(motors[1].disabled, isFalse);
      expect(motors[2].disabled, isTrue);
      expect(m.modelChips.map((WizardChip c) => c.label).take(2).toList(), <String>['GPT-6.1 Sol', 'GPT-6 Astra']);
      expect(m.effortChips.map((WizardChip c) => c.id).toList(),
          <String>['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
    });

    test('blocked routes summary counts like the web (12 listas · 4 bloqueadas)', () {
      final WizardModel m = freshModel();
      expect(m.unavailableSummary, '12 combinaciones listas · 4 bloqueadas (login / gateway)');
      expect(m.blockedRoutes.first.title, 'Codex CLI → Claude');
      expect(m.showUnavailable, isTrue);
    });

    test('picking a model keeps a valid effort, else its default effort', () {
      final WizardModel m = freshModel();
      m.pickEffort('ultra');
      m.pickModel('gpt-6-luna'); // no «ultra», default «medium»
      expect(m.sel.effort, 'medium');
      m.pickEffort('low');
      m.pickModel('gpt-6-sol');
      expect(m.sel.effort, 'low');
    });

    test('Claude Code: accounts with titles; logged-out ones disabled; first model and default effort', () {
      final WizardModel m = freshModel()..pickHarness('claude');
      expect(m.sel.motor, 'claude');
      expect(m.sel.routeId, 'claude:claude');
      expect(m.sel.model, 'claude-fable-5-2');
      expect(m.sel.effort, 'high');
      expect(m.showHarnessAccounts, isTrue);
      final List<WizardChip> acc = m.harnessAccountChips;
      expect(acc.map((WizardChip c) => c.id).toList(), <String>['main', 'work', 'side', 'extra']);
      expect(acc[1].disabled, isTrue);
      expect(acc[1].title, 'requiere login');
      expect(acc[0].title, 'person@example.com');
      expect(m.showMotorAccounts, isFalse);
      m.pickHarnessAccount('side');
      expect(m.sel.motorAccount, 'side', reason: 'same provider: the motor account follows');
    });

    test('Claude Code → Codex motor bills a separate motor account', () {
      final WizardModel m = freshModel()
        ..pickHarness('claude')
        ..pickHarnessAccount('side')
        ..pickMotor('codex');
      expect(m.sel.routeId, 'claude:codex');
      expect(m.showMotorAccounts, isTrue);
      expect(m.motorAccountChips.map((WizardChip c) => c.id).toList(), <String>['main']);
      expect(m.sel.motorAccount, 'main');
      expect(m.sel.harnessAccount, 'side');
      expect(m.note, 'Claude Code → Codex · gpt-6.1-sol · high');
    });

    test('ACP: no motor until one is chosen, then motor accounts by «selectable»', () {
      final WizardModel m = freshModel()..pickHarness('acp');
      expect(m.sel.routeId, '');
      expect(m.canCreate, isFalse);
      expect(m.note, 'combinación no disponible');
      expect(m.createTitle, 'Esta combinación no está disponible — elige un motor habilitado');
      expect(m.showHarnessAccounts, isFalse);
      m.pickMotor('claude');
      expect(m.sel.routeId, 'acp:claude');
      expect(m.showMotorAccounts, isTrue);
      final List<WizardChip> acc = m.motorAccountChips;
      expect(acc.firstWhere((WizardChip c) => c.id == 'side').disabled, isFalse,
          reason: 'ACP uses «selectable», not «motorSelectable»');
      expect(m.canCreate, isTrue);
    });

    test('Shell hides motor/model/effort/accounts and posts agent shell', () {
      final WizardModel m = freshModel()..pickHarness('shell');
      expect(m.isShell, isTrue);
      expect(m.note, '');
      expect(m.canCreate, isTrue);
      expect(m.showUnavailable, isFalse);
      expect(m.createBody(cwd: '/a', danger: false), <String, Object?>{'cwd': '/a', 'agent': 'shell', 'danger': false});
    });

    test('create body is the web payload', () {
      final WizardModel m = freshModel();
      expect(m.createBody(cwd: '/srv/x', danger: true), <String, Object?>{
        'cwd': '/srv/x',
        'routeId': 'codex:codex',
        'model': 'gpt-6.1-sol',
        'effort': 'high',
        'harnessAccount': 'main',
        'motorAccount': 'main',
        'danger': true,
      });
      expect(m.createBody(cwd: '/srv/x', danger: false, shell: true),
          <String, Object?>{'cwd': '/srv/x', 'agent': 'shell', 'danger': false});
    });

    test('active optimisation plan variant sets motor, model and effort', () {
      final WizardModel m = freshModel()..pickHarness('claude');
      m.applyPlanVariant(<String, Object?>{
        'routeId': 'claude:codex', 'model': 'gpt-5.6-terra', 'effort': 'medium', 'selectable': true,
      });
      m.selectDefaults();
      m.normalize();
      expect(m.sel.motor, 'codex');
      expect(m.sel.model, 'gpt-5.6-terra');
      expect(m.sel.effort, 'medium');
      expect(m.sel.routeId, 'claude:codex');
      m.applyPlanVariant(<String, Object?>{'routeId': 'grok:grok', 'model': 'x', 'selectable': false});
      expect(m.sel.model, 'gpt-5.6-terra');
    });

    test('a profile keeps its fields; validate reports what is wrong', () {
      final WizardModel m = freshModel();
      m.applyProfileDraft('p1', <String, Object?>{
        'agent': 'claude', 'motor': 'claude', 'model': 'claude-opus-5-5', 'effort': 'max', 'harnessAccount': 'work',
      });
      expect(m.sel.profileId, 'p1');
      expect(m.sel.harness, 'claude');
      expect(m.sel.harnessAccount, 'work', reason: 'ensure_account does not touch a profile');
      expect(m.profileError, 'Selecciona una cuenta con sesión iniciada');
      expect(m.note, 'Selecciona una cuenta con sesión iniciada');
      expect(m.canCreate, isFalse);
      m.pickHarnessAccount('main');
      expect(m.profileError, '');
      expect(m.createBody(cwd: '/a', danger: false)['profileId'], 'p1');
    });
  });

  group('WizardPaths', () {
    test('crumbs, short home, recent folders, create error text', () {
      expect(WizardPaths.crumbs('/srv/dev/code'), <(String, String)>[
        ('srv', '/srv'), ('dev', '/srv/dev'), ('code', '/srv/dev/code'),
      ]);
      expect(WizardPaths.shortHome('/srv/dev/nonexist/xyz', '/srv/dev'), '~/nonexist/xyz');
      expect(WizardPaths.shortHome('/opt/x', '/srv/dev'), '/opt/x');
      expect(WizardPaths.recentDirs(<String>['/b', '', '/a', '/b']), <String>['/a', '/b']);
      expect(WizardPaths.createErrorText('Esa carpeta no existe todavía.'),
          'Esa carpeta no existe — usa el botón Crear carpeta de abajo.');
      expect(WizardPaths.createErrorText('carpeta invalida'), 'carpeta invalida');
    });
  });

  group('NewSession panel', () {
    late List<(String, Map<String, Object?>)> posts;
    late http.Response Function(Map<String, Object?> body) onCreate;

    Future<String?> Function() open(WidgetTester tester) {
      String? result;
      bool done = false;
      tester.view.physicalSize = const Size(412, 915);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      return () async {
        await tester.pumpWidget(MaterialApp(
          theme: buildTheme(),
          home: Builder(
            builder: (BuildContext context) => Center(
              child: TextButton(
                onPressed: () async {
                  result = await showNewSession(context);
                  done = true;
                },
                child: const Text('abrir'),
              ),
            ),
          ),
        ));
        await tester.tap(find.text('abrir'));
        await tester.pumpAndSettle();
        return done ? result : '__open__';
      };
    }

    setUp(() {
      resetNewSessionMemory();
      sl.reset();
      posts = <(String, Map<String, Object?>)>[];
      onCreate = (_) => jsonResponse(<String, Object?>{'ok': true, 'session': 'term-r123', 'label': 'portal'});
      final MockClient client = MockClient((http.Request r) async {
        final Map<String, Object?> body =
            r.method == 'POST' ? (jsonDecode(utf8.decode(r.bodyBytes)) as Map<String, Object?>) : <String, Object?>{};
        if (r.method == 'POST') posts.add((r.url.path, body));
        switch (r.url.path) {
          case '/providers':
            return http.Response.bytes(utf8.encode(providersJson), 200,
                headers: <String, String>{'content-type': 'application/json; charset=utf-8'});
          case '/model-tiers':
            return jsonResponse(<String, Object?>{'providers': <Object?>[]});
          case '/state':
            if (r.url.queryParameters['live'] != '1') return jsonResponse(<String, Object?>{'error': 'live'}, 400);
            return jsonResponse(<Object?>[
              <String, Object?>{'session': 'term-r1', 'pane': '%3', 'cwd': '/srv/dev/code/acme/portal'},
              <String, Object?>{'session': 'term-r2', 'pane': '%4', 'cwd': '/srv/dev/code/acme/console'},
            ]);
          case '/active-tab':
            return jsonResponse(<String, Object?>{'session': 'term-r1', 'pane': '%3'});
          case '/optimization/plans':
            return jsonResponse(<String, Object?>{'plans': <Object?>[], 'active': ''});
          case '/fs/dirs':
            final String p = r.url.queryParameters['path'] ?? '';
            return jsonResponse(p.contains('nonexist') ? missingDirs : portalDirs);
          case '/fs/mkdir':
            return jsonResponse(<String, Object?>{'ok': true, 'path': body['path']});
          case '/session-new':
            return onCreate(body);
        }
        return jsonResponse(<String, Object?>{'error': 'Not found'}, 404);
      });
      final MemoryCredentialStore creds = MemoryCredentialStore()
        ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
      final Connection connection =
          Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
      sl.register<Connection>(connection);
      return connection.load();
    });

    tearDown(sl.reset);

    testWidgets('shows every section of the web modal with the active folder', (WidgetTester tester) async {
      await open(tester)();
      expect(find.text('Nueva sesión'), findsOneWidget);
      for (final String label in <String>['CARPETA DEL PROYECTO', 'HARNESS', 'MOTOR', 'MODELO', 'PENSAMIENTO', 'CUENTA DEL HARNESS']) {
        expect(find.text(label), findsOneWidget, reason: label);
      }
      expect(find.text('/srv/dev/code/acme/portal'), findsOneWidget);
      expect(find.text('portal'), findsOneWidget); // last crumb
      expect(find.text('apps'), findsOneWidget);
      expect(find.text('carpeta existente'), findsOneWidget);
      expect(find.text('Codex CLI'), findsOneWidget);
      expect(find.text('GPT-6.1 Sol'), findsOneWidget);
      expect(find.text('▶ 12 combinaciones listas · 4 bloqueadas (login / gateway)'), findsOneWidget);
      expect(find.text('sin aprobaciones (dangerously)'), findsOneWidget);
      expect(find.text('Codex CLI → Codex · gpt-6.1-sol · high'), findsOneWidget);
      for (final String b in <String>['Cancelar', 'Sin perfil', 'Elegir perfil', 'Quitar perfil', 'Solo terminal', 'Crear']) {
        expect(find.text(b), findsOneWidget, reason: b);
      }
      expect(posts, isEmpty);
    });

    testWidgets('Crear posts the web body and completes with the session name', (WidgetTester tester) async {
      String? result = '__open__';
      bool closed = false;
      tester.view.physicalSize = const Size(412, 915);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (BuildContext context) => Center(
            child: TextButton(
              onPressed: () async {
                result = await showNewSession(context);
                closed = true;
              },
              child: const Text('abrir'),
            ),
          ),
        ),
      ));
      await tester.tap(find.text('abrir'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('sin aprobaciones (dangerously)'));
      await tester.tap(find.text('sin aprobaciones (dangerously)'));
      await tester.pump();
      await tester.ensureVisible(find.text('Crear'));
      await tester.tap(find.text('Crear'));
      await tester.pumpAndSettle();
      expect(closed, isTrue);
      expect(result, 'term-r123');
      expect(posts.single.$1, '/session-new');
      expect(posts.single.$2, <String, Object?>{
        'cwd': '/srv/dev/code/acme/portal',
        'routeId': 'codex:codex',
        'model': 'gpt-6.1-sol',
        'effort': 'high',
        'harnessAccount': 'main',
        'motorAccount': 'main',
        'danger': true,
      });
    });

    testWidgets('a missing folder shows the web toast and the «Crear carpeta» button', (WidgetTester tester) async {
      onCreate = (_) => jsonResponse(
            <String, Object?>{'error': 'Esa carpeta no existe todavía.', 'code': 'cwd_missing', 'cwd': '/x'},
            409,
          );
      await open(tester)();
      await tester.enterText(find.byKey(const ValueKey<String>('ns-cwd')), '/srv/dev/nonexist/xyz');
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expect(find.text('~/nonexist/xyz no existe'), findsOneWidget);
      expect(find.text('Crear carpeta'), findsOneWidget);
      await tester.ensureVisible(find.text('Crear'));
      await tester.tap(find.text('Crear'));
      await tester.pumpAndSettle();
      expect(find.text('Esa carpeta no existe — usa el botón Crear carpeta de abajo.'), findsOneWidget);
      expect(find.text('Nueva sesión'), findsOneWidget, reason: 'the panel stays open');
      await tester.pump(const Duration(seconds: 5)); // the toast goes away
      await tester.ensureVisible(find.text('Crear carpeta'));
      await tester.tap(find.text('Crear carpeta'));
      await tester.pumpAndSettle();
      expect(posts.last.$1, '/fs/mkdir');
      expect(posts.last.$2, <String, Object?>{'path': '/srv/dev/nonexist/xyz'});
      expect(find.text('Carpeta creada: ~/nonexist/xyz'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
    });

    testWidgets('a 400 shows the server text', (WidgetTester tester) async {
      onCreate = (_) => jsonResponse(<String, Object?>{'error': 'carpeta invalida'}, 400);
      await open(tester)();
      await tester.ensureVisible(find.text('Crear'));
      await tester.tap(find.text('Crear'));
      await tester.pumpAndSettle();
      expect(find.text('carpeta invalida'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
      expect(find.text('carpeta invalida'), findsNothing);
    });

    testWidgets('Solo terminal posts a plain shell; picking Shell hides the agent rows', (WidgetTester tester) async {
      await open(tester)();
      await tester.ensureVisible(find.text('Solo terminal'));
      await tester.tap(find.text('Solo terminal'));
      await tester.pumpAndSettle();
      expect(posts.single.$2, <String, Object?>{
        'cwd': '/srv/dev/code/acme/portal', 'agent': 'shell', 'danger': false,
      });
    });

    testWidgets('choosing Claude Code refilters motors, models and accounts', (WidgetTester tester) async {
      await open(tester)();
      await tester.tap(find.text('Claude Code'));
      await tester.pumpAndSettle();
      expect(find.text('Fable 5.2'), findsOneWidget);
      expect(find.text('Claude Code → Claude · claude-fable-5-2 · high'), findsOneWidget);
      await tester.tap(find.text('Shell'));
      await tester.pumpAndSettle();
      expect(find.text('MOTOR'), findsNothing);
      expect(find.text('MODELO'), findsNothing);
      expect(find.text('CUENTA DEL HARNESS'), findsNothing);
    });

    testWidgets('Cancelar completes with null and creates nothing', (WidgetTester tester) async {
      String? result = '__open__';
      bool closed = false;
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (BuildContext context) => Center(
            child: TextButton(
              onPressed: () async {
                result = await showNewSession(context);
                closed = true;
              },
              child: const Text('abrir'),
            ),
          ),
        ),
      ));
      await tester.tap(find.text('abrir'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Cancelar'));
      await tester.tap(find.text('Cancelar'));
      await tester.pumpAndSettle();
      expect(closed, isTrue);
      expect(result, isNull);
      expect(posts, isEmpty);
    });
  });
}
