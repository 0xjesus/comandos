import 'dart:async';
import 'dart:convert';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/api/api_errors.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/models/provider_registry.dart';
import 'package:comandos_mobile/features/new_session/folder_search.dart';
import 'package:comandos_mobile/features/new_session/new_session.dart';
import 'package:comandos_mobile/features/new_session/provider_mark.dart';
import 'package:comandos_mobile/features/new_session/wizard_model.dart';
import 'package:comandos_mobile/features/new_session/wizard_widgets.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:comandos_mobile/ui/web_kit.dart';
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

  group('Models: long catalogs', () {
    test('more than 8 models: first 8 chips, the chosen one stays visible, search by id and name', () {
      final WizardModel m = freshModel()
        ..pickHarness('acp')
        ..pickMotor('opencode');
      expect(m.modelList.length, 10);
      expect(m.hasModelSearch, isTrue);
      expect(m.visibleModelChips.length, WizardModel.modelChipLimit);
      m.pickModel('groq/qwen/qwen3.8-27b'); // the 10th
      expect(m.visibleModelChips.length, 9);
      expect(m.visibleModelChips.last.id, 'groq/qwen/qwen3.8-27b');
      expect(m.filterModels('GROQ qwen').map((WizardChip c) => c.id).toList(),
          <String>['groq/qwen/qwen3.6-27b', 'groq/qwen/qwen3.8-27b']);
      expect(m.filterModels('Llama 3.3').map((WizardChip c) => c.id).toList(), <String>['groq/llama-3.3-70b-versatile']);
      expect(m.filterModels('').length, 10);
      expect(m.filterModels('nada-asi'), isEmpty);
      final WizardModel codex = freshModel();
      expect(codex.hasModelSearch, isFalse);
      expect(codex.visibleModelChips.length, 7);
    });

    test('a model tag reaches its chip', () {
      final ProviderRegistry reg = ProviderRegistry.tryParse(<String, Object?>{
        'matrixHarnesses': <Object?>['omp'],
        'harnesses': <String, Object?>{
          'omp': <String, Object?>{'label': 'Oh My Pi', 'icon': 'omp'},
        },
        'motors': <String, Object?>{
          'omp': <String, Object?>{
            'label': 'Oh My Pi',
            'icon': 'omp',
            'models': <Object?>[
              <String, Object?>{'id': 'refuseless/glm-5.3', 'name': 'GLM 5.3', 'tag': 'nuevo'},
              <String, Object?>{'id': 'b', 'name': 'B'},
            ],
          },
        },
        'matrix': <Object?>[
          <String, Object?>{'id': 'omp:omp', 'harness': 'omp', 'motor': 'omp', 'selectable': true, 'driver': 'omp'},
        ],
      })!;
      final WizardModel m = WizardModel(reg, WizardSelection.defaults())..pickHarness('omp');
      expect(m.harnessChips.first.label, 'Oh My Pi');
      expect(m.harnessChips.first.icon, 'omp');
      expect(m.modelChips.first.tag, 'nuevo');
      expect(m.modelChips.last.tag, '');
      expect(m.sel.routeId, 'omp:omp');
    });

    test('Oh My Pi has its π everywhere', () {
      ProviderMarks.reset();
      expect(ProviderMarks.resolve('omp', ''), WebIcons.omp);
      expect(ProviderMarks.resolve('omp', 'refuseless/glm-5.3'), WebIcons.omp);
      // Rules from an older server that does not know omp yet.
      ProviderMarks.setRules(<String, Object?>{
        'providers': <Object?>[
          <String, Object?>{'match': 'claude', 'icon': 'anthropic'},
        ],
      });
      expect(ProviderMarks.resolve('omp', ''), WebIcons.omp);
      expect(ProviderMarks.resolve('omp', 'claude-opus-5-5'), WebIcons.anthropic);
      ProviderMarks.reset();
      expect(WebIcons.byName['omp'], WebIcons.omp);
    });
  });

  group('WizardPaths', () {
    test('crumbs, short home, folder name, create error text', () {
      expect(WizardPaths.crumbs('/srv/dev/code'), <(String, String)>[
        ('srv', '/srv'), ('dev', '/srv/dev'), ('code', '/srv/dev/code'),
      ]);
      expect(WizardPaths.shortHome('/srv/dev/nonexist/xyz', '/srv/dev'), '~/nonexist/xyz');
      expect(WizardPaths.shortHome('/opt/x', '/srv/dev'), '/opt/x');
      expect(WizardPaths.folderName('/srv/dev/code/'), 'code');
      expect(WizardPaths.folderName('~/codebase/ComandOS'), 'ComandOS');
      expect(WizardPaths.folderName('/'), '/');
      expect(WizardPaths.createErrorText('Esa carpeta no existe todavía.'),
          'Esa carpeta no existe — usa el botón Crear carpeta.');
      expect(WizardPaths.createErrorText('carpeta invalida'), 'carpeta invalida');
    });

    test('recent folders: once each, most recently active first', () {
      expect(
        WizardPaths.recentFolders(<Map<String, Object?>>[
          <String, Object?>{'cwd': '/a', 'ts': 10},
          <String, Object?>{'cwd': '/b', 'ts': 30},
          <String, Object?>{'cwd': ''},
          <String, Object?>{'cwd': '/a', 'ts': 50},
          <String, Object?>{'cwd': '/c'},
        ]),
        <String>['/a', '/b', '/c'],
      );
    });

    test('path text vs search text; match score and highlight', () {
      expect(WizardPaths.isPathQuery('~/code'), isTrue);
      expect(WizardPaths.isPathQuery('/srv'), isTrue);
      expect(WizardPaths.isPathQuery('comandos'), isFalse);
      expect(WizardPaths.matchScore('comandos', 'ComandOS'), 0);
      expect(WizardPaths.matchScore('coma', 'ComandOS'), 1);
      expect(WizardPaths.matchScore('web', 'comandos-web'), 2);
      expect(WizardPaths.matchScore('and', 'ComandOS'), 3);
      expect(WizardPaths.matchScore('cmdos', 'ComandOS'), 4);
      expect(WizardPaths.matchScore('zz', 'ComandOS'), isNull);
      expect(WizardPaths.highlight('ComandOS', 'mand'), <(String, bool)>[('Co', false), ('mand', true), ('OS', false)]);
      expect(WizardPaths.highlight('ComandOS', 'cos'),
          <(String, bool)>[('Co', true), ('mandO', false), ('S', true)]);
      expect(WizardPaths.highlight('ComandOS', 'codebase/coma'), <(String, bool)>[('Coma', true), ('ndOS', false)]);
      expect(WizardPaths.highlight('ComandOS', ''), <(String, bool)>[('ComandOS', false)]);
      expect(WizardPaths.highlight('ComandOS', 'xyz'), <(String, bool)>[('ComandOS', false)]);
    });
  });

  group('FolderSearch', () {
    late Map<String, List<(Map<String, String>, Completer<Object?>)>> calls;
    late FolderSearch f;

    Future<Object?> get(String path, Map<String, String> q) {
      final Completer<Object?> c = Completer<Object?>();
      (calls[path] ??= <(Map<String, String>, Completer<Object?>)>[]).add((q, c));
      return c.future;
    }

    Future<void> settle() async {
      for (int i = 0; i < 5; i++) {
        await Future<void>.delayed(Duration.zero);
      }
    }

    Map<String, Object?> found(String q, List<String> names) => <String, Object?>{
          'q': q,
          'home': '/srv/dev',
          'results': <Object?>[
            for (final String n in names)
              <String, Object?>{'path': '/srv/dev/code/$n', 'short': '~/code/$n', 'name': n, 'repo': n == 'portal'},
          ],
        };

    setUp(() {
      calls = <String, List<(Map<String, String>, Completer<Object?>)>>{};
      f = FolderSearch(get: get, debounce: Duration.zero, recent: <String>['/srv/dev/code/acme/console']);
    });

    tearDown(() => f.dispose());

    test('views follow the text: recent, search, path, picked', () async {
      expect(f.view, FolderView.recent);
      expect(f.recentEntries.single.name, 'console');
      f.setQuery('port');
      expect(f.view, FolderView.search);
      f.setQuery('~/code');
      expect(f.view, FolderView.path);
      await settle();
      expect(calls['/fs/find'], isNull, reason: 'debounced: only the last text is asked for');
      expect(calls['/fs/dirs']!.single.$1, <String, String>{'path': '~/code'});
      f.setQuery('');
      expect(f.view, FolderView.recent);
      f.pick('/srv/dev/code/portal');
      expect(f.view, FolderView.picked);
      expect(f.pickedName, 'portal');
      expect(calls['/fs/dirs']!.last.$1, <String, String>{'path': '/srv/dev/code/portal'});
      f.change();
      expect(f.view, FolderView.recent);
      expect(f.editing, isTrue);
      expect(f.picked, '/srv/dev/code/portal', reason: '«Cambiar» keeps the folder until another is chosen');
    });

    test('only the newest reply paints; older rows stay while the next one loads', () async {
      f.setQuery('po');
      await settle();
      f.setQuery('portal');
      await settle();
      final List<(Map<String, String>, Completer<Object?>)> finds = calls['/fs/find']!;
      expect(finds.map(((Map<String, String>, Completer<Object?>) c) => c.$1['q']).toList(), <String>['po', 'portal']);
      expect(finds.first.$1['limit'], '40');
      finds[1].$2.complete(found('portal', <String>['portal']));
      await settle();
      expect(f.results.map((FolderEntry e) => e.name).toList(), <String>['portal']);
      expect(f.results.single.repo, isTrue);
      expect(f.loading, isFalse);
      finds[0].$2.complete(found('po', <String>['portal', 'poster', 'pocket']));
      await settle();
      expect(f.results.map((FolderEntry e) => e.name).toList(), <String>['portal'], reason: 'stale reply ignored');
      f.setQuery('con');
      await settle();
      expect(f.loading, isTrue);
      expect(f.results.single.name, 'portal', reason: 'kept visible while loading');
      calls['/fs/find']!.last.$2.complete(found('con', <String>['console', 'conf']));
      await settle();
      // The live session's console first; another «console» folder after it.
      expect(f.results.map((FolderEntry e) => (e.short, e.recent)).toList(),
          <(String, bool)>[('~/code/acme/console', true), ('~/code/console', false), ('~/code/conf', false)]);
    });

    test('a pick beats a search still on its way', () async {
      f.setQuery('po');
      await settle();
      f.pick('/srv/dev/code/acme/console');
      calls['/fs/find']!.single.$2.complete(found('po', <String>['portal']));
      await settle();
      expect(f.results, isEmpty);
      expect(f.view, FolderView.picked);
    });

    test('recent folders that match go first, with the badge, even when not found remotely', () async {
      f.setQuery('cons');
      await settle();
      calls['/fs/find']!.single.$2.complete(found('cons', <String>['constants']));
      await settle();
      expect(f.results.map((FolderEntry e) => (e.short, e.recent)).toList(),
          <(String, bool)>[('~/code/acme/console', true), ('~/code/constants', false)]);
    });

    test('an older server without /fs/find: searches ~/codebase and ~ instead', () async {
      f.setQuery('acm');
      await settle();
      calls['/fs/find']!.single.$2.completeError(const NotFoundException());
      await settle();
      expect(f.findUnavailable, isTrue);
      final List<(Map<String, String>, Completer<Object?>)> dirs = calls['/fs/dirs']!;
      expect(dirs.first.$1['path'], '~/codebase');
      dirs.first.$2.complete(<String, Object?>{
        'path': '/srv/dev/codebase', 'base': '/srv/dev/codebase', 'exists': true, 'home': '/srv/dev',
        'dirs': <Object?>[
          <String, Object?>{'name': 'acme', 'path': '/srv/dev/codebase/acme'},
          <String, Object?>{'name': 'other', 'path': '/srv/dev/codebase/other'},
        ],
      });
      await settle();
      calls['/fs/dirs']![1].$2.complete(<String, Object?>{
        'path': '/srv/dev', 'base': '/srv/dev', 'exists': true, 'home': '/srv/dev',
        'dirs': <Object?>[
          <String, Object?>{'name': 'codebase', 'path': '/srv/dev/codebase'},
          <String, Object?>{'name': 'Academia', 'path': '/srv/dev/Academia'},
        ],
      });
      await settle();
      expect(f.results.map((FolderEntry e) => e.short).toList(), <String>['~/codebase/acme', '~/Academia'],
          reason: 'a recent folder only goes first when its name matches');
      // Next queries go straight to the fallback list (no second /fs/find).
      f.setQuery('other');
      await settle();
      expect(calls['/fs/find']!.length, 1);
      expect(f.results.map((FolderEntry e) => e.name).toList(), <String>['other']);
    });

    test('a typed path that does not exist lists the subfolders that start like it', () async {
      f.setQuery('~/codebase/Com');
      await settle();
      calls['/fs/dirs']!.single.$2.complete(<String, Object?>{
        'path': '/srv/dev/codebase/Com', 'base': '/srv/dev/codebase', 'exists': false, 'missing': 'Com',
        'home': '/srv/dev',
        'dirs': <Object?>[
          <String, Object?>{'name': 'ComandOS', 'path': '/srv/dev/codebase/ComandOS'},
          <String, Object?>{'name': 'acme', 'path': '/srv/dev/codebase/acme'},
        ],
      });
      await settle();
      expect(f.pathDirs, <(String, String)>[('ComandOS', '/srv/dev/codebase/ComandOS')]);
      expect(f.home, '/srv/dev');
    });
  });

  group('NewSession panel', () {
    late List<(String, Map<String, Object?>)> posts;
    late List<Uri> gets;
    late http.Response Function(Map<String, Object?> body) onCreate;
    late bool findAvailable;
    late bool activeTab;

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

    Finder byKey(String k) => find.byKey(ValueKey<String>(k));

    Future<void> tapKey(WidgetTester tester, String k) async {
      await tester.ensureVisible(byKey(k));
      await tester.pumpAndSettle();
      await tester.tap(byKey(k));
      await tester.pumpAndSettle();
    }

    Future<void> typeFolder(WidgetTester tester, String text) async {
      await tester.enterText(byKey('ns-cwd'), text);
      await tester.pump(const Duration(milliseconds: 150));
      await tester.pumpAndSettle();
    }

    setUp(() {
      resetNewSessionMemory();
      sl.reset();
      posts = <(String, Map<String, Object?>)>[];
      gets = <Uri>[];
      findAvailable = true;
      activeTab = true;
      onCreate = (_) => jsonResponse(<String, Object?>{'ok': true, 'session': 'term-r123', 'label': 'portal'});
      final MockClient client = MockClient((http.Request r) async {
        final Map<String, Object?> body =
            r.method == 'POST' ? (jsonDecode(utf8.decode(r.bodyBytes)) as Map<String, Object?>) : <String, Object?>{};
        if (r.method == 'POST') posts.add((r.url.path, body));
        if (r.method == 'GET') gets.add(r.url);
        switch (r.url.path) {
          case '/providers':
            return http.Response.bytes(utf8.encode(providersJson), 200,
                headers: <String, String>{'content-type': 'application/json; charset=utf-8'});
          case '/model-tiers':
            return jsonResponse(<String, Object?>{'providers': <Object?>[]});
          case '/state':
            if (r.url.queryParameters['live'] != '1') return jsonResponse(<String, Object?>{'error': 'live'}, 400);
            return jsonResponse(<Object?>[
              <String, Object?>{'session': 'term-r1', 'pane': '%3', 'cwd': '/srv/dev/code/acme/portal', 'ts': 100},
              <String, Object?>{'session': 'term-r2', 'pane': '%4', 'cwd': '/srv/dev/code/acme/console', 'ts': 200},
            ]);
          case '/active-tab':
            return activeTab
                ? jsonResponse(<String, Object?>{'session': 'term-r1', 'pane': '%3'})
                : jsonResponse(<String, Object?>{});
          case '/optimization/plans':
            return jsonResponse(<String, Object?>{'plans': <Object?>[], 'active': ''});
          case '/fs/find':
            if (!findAvailable) return jsonResponse(<String, Object?>{'error': 'Not found'}, 404);
            final String q = r.url.queryParameters['q'] ?? '';
            return jsonResponse(<String, Object?>{
              'q': q,
              'home': '/srv/dev',
              'indexed': 3,
              'ms': 1,
              'results': <Object?>[
                if ('console'.contains(q))
                  <String, Object?>{
                    'path': '/srv/dev/code/acme/console', 'short': '~/code/acme/console', 'name': 'console',
                    'repo': true, 'score': 9,
                  },
                if ('constants'.contains(q))
                  <String, Object?>{
                    'path': '/srv/dev/code/constants', 'short': '~/code/constants', 'name': 'constants',
                    'repo': false, 'score': 5,
                  },
              ],
            });
          case '/fs/dirs':
            final String p = r.url.queryParameters['path'] ?? '';
            if (p.contains('nonexist')) return jsonResponse(missingDirs);
            if (p == '~' || p == '~/codebase') {
              return jsonResponse(<String, Object?>{
                'path': '/srv/dev', 'base': '/srv/dev', 'exists': true, 'home': '/srv/dev',
                'dirs': <Object?>[
                  <String, Object?>{'name': 'code', 'path': '/srv/dev/code'},
                  <String, Object?>{'name': 'contabilidad', 'path': '/srv/dev/contabilidad'},
                ],
              });
            }
            return jsonResponse(portalDirs);
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

    testWidgets('a full page: the current tab\'s folder chosen, agent, model, fixed footer', (WidgetTester tester) async {
      await open(tester)();
      expect(find.text('Nueva sesión'), findsOneWidget);
      for (final String label in <String>['CARPETA', 'AGENTE', 'MOTOR', 'MODELO', 'PENSAMIENTO', 'CUENTA DEL HARNESS']) {
        expect(find.text(label), findsOneWidget, reason: label);
      }
      // The pill and the subfolders of the chosen folder.
      expect(byKey('ns-picked'), findsOneWidget);
      expect(find.descendant(of: byKey('ns-picked'), matching: find.text('portal')), findsOneWidget);
      expect(find.descendant(of: byKey('ns-picked'), matching: find.text('~/code/acme/portal')), findsOneWidget);
      expect(find.text('Cambiar'), findsOneWidget);
      expect(find.text('Subcarpetas de portal'), findsOneWidget);
      expect(find.text('apps'), findsOneWidget);
      expect(find.text('docs'), findsOneWidget);
      expect(byKey('ns-cwd'), findsNothing, reason: 'the field collapsed into the pill');
      // Agent chips, model, footer.
      expect(find.text('Codex CLI'), findsOneWidget);
      expect(find.text('GPT-6.1 Sol'), findsOneWidget);
      expect(find.text('Más opciones'), findsOneWidget);
      expect(find.text('sin aprobaciones (dangerously)'), findsNothing, reason: 'inside «Más opciones»');
      expect(find.text('Codex CLI → Codex · gpt-6.1-sol · high'), findsOneWidget);
      expect(find.text('Solo terminal'), findsOneWidget);
      expect(find.text('Crear en portal'), findsOneWidget);
      // The footer sits at the bottom of the screen, the header at the top.
      expect(tester.getBottomLeft(byKey('ns-go')).dy, greaterThan(915 - 80));
      expect(tester.getTopLeft(find.text('Nueva sesión')).dy, lessThan(40));
      expect(tester.getSize(byKey('ns-go')).height, greaterThanOrEqualTo(44));
      expect(posts, isEmpty);
    });

    testWidgets('«Crear en …» posts the web body (danger from «Más opciones») and completes with the session',
        (WidgetTester tester) async {
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
      await tapKey(tester, 'ns-more');
      expect(find.text('Elegir perfil'), findsOneWidget);
      expect(find.text('Sin perfil'), findsOneWidget);
      expect(find.text('12 combinaciones listas · 4 bloqueadas (login / gateway)'), findsOneWidget);
      await tapKey(tester, 'ns-unavailable');
      expect(find.text('Codex CLI → Claude'), findsOneWidget);
      await tapKey(tester, 'ns-danger');
      expect(find.text('sin aprobaciones'), findsOneWidget, reason: 'the flag shows on the section head');
      await tester.tap(byKey('ns-go'));
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

    testWidgets('search mode switching: recent → /fs/find → path → pick a subfolder', (WidgetTester tester) async {
      await open(tester)();
      await tapKey(tester, 'ns-change');
      expect(byKey('ns-cwd'), findsOneWidget);
      expect(find.text('Busca un proyecto o escribe una ruta'), findsOneWidget);
      expect(find.text('Recientes'), findsOneWidget);
      // Most recently active first.
      final double console = tester.getTopLeft(find.text('console')).dy;
      final double portal = tester.getTopLeft(find.text('portal')).dy;
      expect(console, lessThan(portal));

      await typeFolder(tester, 'cons');
      final List<Uri> finds = gets.where((Uri u) => u.path == '/fs/find').toList();
      expect(finds.length, 1, reason: 'debounced: one request for the whole word');
      expect(finds.single.queryParameters, <String, String>{'q': 'cons', 'limit': '40'});
      expect(find.text('Recientes'), findsNothing);
      expect(find.text('~/code/acme/console'), findsOneWidget);
      expect(find.text('~/code/constants'), findsOneWidget);
      expect(find.text('reciente'), findsOneWidget, reason: 'console is a live session folder');
      expect(tester.getTopLeft(find.text('~/code/acme/console')).dy,
          lessThan(tester.getTopLeft(find.text('~/code/constants')).dy));

      await typeFolder(tester, '~/code/acme/portal');
      expect(gets.last.path, '/fs/dirs');
      expect(gets.last.queryParameters['path'], '~/code/acme/portal');
      expect(byKey('ns-crumbs'), findsOneWidget);
      expect(find.text('Usar portal'), findsOneWidget);
      expect(find.text('apps'), findsOneWidget);

      await tester.tap(find.text('apps'));
      await tester.pumpAndSettle();
      expect(byKey('ns-picked'), findsOneWidget);
      expect(find.text('Crear en apps'), findsOneWidget);
      expect(gets.last.queryParameters['path'], '/srv/dev/code/acme/portal/apps');

      // Clear on an empty field goes back to the chosen folder.
      await tapKey(tester, 'ns-change');
      await tapKey(tester, 'ns-cwd-clear');
      expect(byKey('ns-picked'), findsOneWidget);
      expect(find.text('Crear en apps'), findsOneWidget);
    });

    testWidgets('without /fs/find (older server) the search falls back to ~ and ~/codebase', (WidgetTester tester) async {
      findAvailable = false;
      await open(tester)();
      await tapKey(tester, 'ns-change');
      await typeFolder(tester, 'conta');
      expect(find.textContaining('aún no tiene la búsqueda completa'), findsOneWidget);
      expect(find.text('~/contabilidad'), findsOneWidget);
      await tester.tap(find.text('~/contabilidad'));
      await tester.pumpAndSettle();
      expect(find.text('Crear en contabilidad'), findsOneWidget);
    });

    testWidgets('a missing folder: «no existe», Crear carpeta, and the create error toast', (WidgetTester tester) async {
      onCreate = (_) => jsonResponse(
            <String, Object?>{'error': 'Esa carpeta no existe todavía.', 'code': 'cwd_missing', 'cwd': '/x'},
            409,
          );
      await open(tester)();
      await tapKey(tester, 'ns-change');
      await typeFolder(tester, '/srv/dev/nonexist/xyz');
      expect(find.text('~/nonexist/xyz no existe'), findsOneWidget);
      // Enter chooses the typed path as it is.
      await tester.testTextInput.receiveAction(TextInputAction.search);
      await tester.pumpAndSettle();
      expect(byKey('ns-picked'), findsOneWidget);
      expect(find.text('~/nonexist/xyz no existe'), findsOneWidget);
      await tester.tap(byKey('ns-go'));
      await tester.pumpAndSettle();
      expect(find.text('Esa carpeta no existe — usa el botón Crear carpeta.'), findsOneWidget);
      expect(find.text('Nueva sesión'), findsOneWidget, reason: 'the page stays open');
      await tester.pump(const Duration(seconds: 5)); // the toast goes away
      await tapKey(tester, 'ns-mkdir');
      expect(posts.last.$1, '/fs/mkdir');
      expect(posts.last.$2, <String, Object?>{'path': '/srv/dev/nonexist/xyz'});
      expect(find.text('Carpeta creada: ~/nonexist/xyz'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
    });

    testWidgets('no folder yet: «Crear sesión» disabled, recent folders shown', (WidgetTester tester) async {
      activeTab = false;
      await open(tester)();
      expect(byKey('ns-picked'), findsNothing);
      expect(find.text('Recientes'), findsOneWidget);
      expect(find.text('Crear sesión'), findsOneWidget);
      expect(find.text('Elige una carpeta para crear la sesión'), findsOneWidget);
      await tester.tap(byKey('ns-go'));
      await tester.tap(byKey('ns-term'));
      await tester.pumpAndSettle();
      expect(posts, isEmpty);
      await tester.tap(find.text('console'));
      await tester.pumpAndSettle();
      expect(find.text('Crear en console'), findsOneWidget);
    });

    testWidgets('a 400 shows the server text', (WidgetTester tester) async {
      onCreate = (_) => jsonResponse(<String, Object?>{'error': 'carpeta invalida'}, 400);
      await open(tester)();
      await tester.tap(byKey('ns-go'));
      await tester.pumpAndSettle();
      expect(find.text('carpeta invalida'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
      expect(find.text('carpeta invalida'), findsNothing);
    });

    testWidgets('Solo terminal posts a plain shell', (WidgetTester tester) async {
      await open(tester)();
      await tester.tap(find.text('Solo terminal'));
      await tester.pumpAndSettle();
      expect(posts.single.$2, <String, Object?>{
        'cwd': '/srv/dev/code/acme/portal', 'agent': 'shell', 'danger': false,
      });
    });

    testWidgets('choosing Claude Code refilters motors, models and accounts; Shell hides them', (WidgetTester tester) async {
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

    testWidgets('«Buscar modelo» filters a long catalog and picks from it', (WidgetTester tester) async {
      await open(tester)();
      await tester.tap(find.text('ComandOS ACP'));
      await tester.pumpAndSettle();
      await tester.tap(find.descendant(of: byKey('ns-motor'), matching: byKey('opencode')));
      await tester.pumpAndSettle();
      expect(find.text('Buscar modelo (10)'), findsOneWidget);
      expect(find.text('Qwen3.8 27B (Groq)'), findsNothing, reason: 'only the first 8 as chips');
      await tapKey(tester, 'ns-model-search-open');
      expect(find.byType(NsModelRow), findsNWidgets(10));
      await tester.enterText(byKey('ns-model-q'), 'groq QWEN');
      await tester.pumpAndSettle();
      expect(find.byType(NsModelRow), findsNWidgets(2));
      await tapKey(tester, 'ns-model-row:groq/qwen/qwen3.8-27b');
      expect(byKey('ns-model-q'), findsNothing, reason: 'the search closes on a pick');
      expect(find.text('Qwen3.8 27B (Groq)'), findsOneWidget, reason: 'the chosen model joins the chips');
      expect(find.text('ComandOS ACP → OpenCode · groq/qwen/qwen3.8-27b'), findsOneWidget);
      await tester.tap(byKey('ns-go'));
      await tester.pumpAndSettle();
      expect(posts.single.$2['routeId'], 'acp:opencode');
      expect(posts.single.$2['model'], 'groq/qwen/qwen3.8-27b');
    });

    testWidgets('the close button completes with null and creates nothing', (WidgetTester tester) async {
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
      await tester.tap(byKey('ns-close'));
      await tester.pumpAndSettle();
      expect(closed, isTrue);
      expect(result, isNull);
      expect(posts, isEmpty);
    });

    testWidgets('a wide screen gets a centred card', (WidgetTester tester) async {
      tester.view.physicalSize = const Size(1280, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (BuildContext context) => Center(
            child: TextButton(onPressed: () => showNewSession(context), child: const Text('abrir')),
          ),
        ),
      ));
      await tester.tap(find.text('abrir'));
      await tester.pumpAndSettle();
      expect(tester.getSize(byKey('ns-body')).width, lessThanOrEqualTo(640));
    });
  });
}
