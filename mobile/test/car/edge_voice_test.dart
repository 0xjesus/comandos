import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:comandos_mobile/features/car/edge_voice.dart';
import 'package:comandos_mobile/features/car/on_device.dart';
import 'package:comandos_mobile/features/voice/speech.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:flutter_test/flutter_test.dart';

class FakePhone implements DeviceSpeech {
  final List<String> said = <String>[];
  @override
  Future<bool> speak(String text, {required SttLanguage language, required double speed}) async {
    said.add(text);
    return true;
  }

  @override
  Future<void> stop() async {}
}

class FakeOut implements AudioOut {
  final List<String> played = <String>[];
  @override
  Future<bool> play(String path, double speed) async {
    played.add(File(path).readAsStringSync());
    return true;
  }

  @override
  Future<void> stop() async {}
  @override
  Future<void> release() async {}
  @override
  Future<void> dispose() async {}
}

void main() {
  late Directory dir;
  late FakePhone phone;
  late FakeOut out;
  setUp(() async {
    dir = await Directory.systemTemp.createTemp('edge');
    phone = FakePhone();
    out = FakeOut();
  });
  tearDown(() => dir.delete(recursive: true));

  EdgeVoice voice(String name, Future<Uint8List> Function(String, String, String)? synth) => EdgeVoice(
        render: () => synth,
        voice: () => name,
        phone: phone,
        dir: () async => dir.path,
        out: out,
      );

  Future<bool> say(EdgeVoice v, String t) => v.speak(t, language: SttLanguage.es, speed: 1.2);

  test('Edge speaks when the desktop renders it, at the chosen voice and speed', () async {
    final List<String> asked = <String>[];
    final EdgeVoice v = voice('es-MX-DaliaNeural', (String t, String n, String r) async {
      asked.add('$n $r $t');
      return Uint8List.fromList(utf8.encode('mp3:$t'));
    });
    expect(await say(v, 'Terminó.'), isTrue);
    expect(asked, <String>['es-MX-DaliaNeural +20% Terminó.']);
    expect(out.played, <String>['mp3:Terminó.']);
    expect(phone.said, isEmpty);
  });

  test('a short phrase is rendered once and then plays from the cache', () async {
    int calls = 0;
    final EdgeVoice v = voice('es-MX-DaliaNeural', (String t, String n, String r) async {
      calls++;
      return Uint8List.fromList(utf8.encode('mp3:$t'));
    });
    await say(v, 'Enviado.');
    await say(v, '¿Lo envío?');
    await say(v, 'Enviado.');
    expect(calls, 2);
    expect(out.played, <String>['mp3:Enviado.', 'mp3:¿Lo envío?', 'mp3:Enviado.']);
  });

  test('Edge failing: the phone speaks at once and keeps speaking for a while', () async {
    int calls = 0;
    final EdgeVoice v = voice('es-MX-DaliaNeural', (String t, String n, String r) async {
      calls++;
      throw const SocketException('down');
    });
    expect(await say(v, 'uno'), isTrue);
    expect(await say(v, 'dos'), isTrue);
    expect(phone.said, <String>['uno', 'dos']);
    expect(calls, 1, reason: 'no new wait for Edge during the cooldown');
  });

  test('the phone voice, or no desktop: never asks Edge', () async {
    bool asked = false;
    Future<Uint8List> synth(String t, String n, String r) async {
      asked = true;
      return Uint8List(1);
    }

    expect(await say(voice('', synth), 'hola'), isTrue);
    expect(await say(voice('es-MX-JorgeNeural', null), 'hola'), isTrue);
    expect(asked, isFalse);
    expect(phone.said, <String>['hola', 'hola']);
  });

  test('edgeRate maps the speed setting', () {
    expect(edgeRate(1.0), '+0%');
    expect(edgeRate(1.5), '+50%');
    expect(edgeRate(0.8), '-20%');
  });
}
