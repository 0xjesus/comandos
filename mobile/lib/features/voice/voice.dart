import 'dart:async';

import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/locator.dart';
import 'dictation.dart';
import 'voice_services.dart';
import 'voice_settings_section.dart';

/// Dictation hook for the chat composer's mic button.
///
/// The chat calls [dictate] when the mic is tapped and appends the returned
/// text to the composer (it does NOT send it; the person reviews first).
/// Return null when the person cancelled or nothing was recognised.
/// The mic button is hidden unless a [VoiceInput] is registered.
abstract class VoiceInput {
  /// Records and transcribes one utterance. Null = cancelled / nothing heard.
  Future<String?> dictate();

  /// Stops a dictation in progress (screen left). Optional.
  Future<void> cancel() async {}
}

/// Text-to-speech hook for "Leer en voz alta" on assistant replies.
/// The menu item is hidden unless a [Speaker] is registered.
abstract class Speaker {
  Future<void> speak(String text);
  Future<void> stop();
}

/// Stub used until the voice feature lands: always "nothing heard".
class NoVoiceInput extends VoiceInput {
  @override
  Future<String?> dictate() async => null;
}

/// Stub speaker: does nothing.
class SilentSpeaker implements Speaker {
  @override
  Future<void> speak(String text) async {}

  @override
  Future<void> stop() async {}
}

/// Forwards to [VoiceServices] without creating anything until used.
class _LazyVoiceInput extends VoiceInput {
  int _n = 0;

  @override
  Future<String?> dictate() async {
    final DictationController d = VoiceServices.instance.dictation;
    final String owner = '_dictate:${_n++}';
    final Completer<String?> got = Completer<String?>();
    Future<void> sink(String text) async {
      if (!got.isCompleted) got.complete(text);
    }

    d.attach(owner, sink);
    try {
      final DictationOutcome o = await d.start(owner);
      if (o is! DictationText) return null;
      return await got.future.timeout(const Duration(seconds: 1), onTimeout: () => o.text);
    } finally {
      d.detach(owner, sink);
    }
  }

  @override
  Future<void> cancel() async => VoiceServices.existing?.dictation.cancel();
}

class _LazySpeaker implements Speaker {
  @override
  Future<void> speak(String text) => VoiceServices.instance.speech.speak(text);

  @override
  Future<void> stop() async => VoiceServices.existing?.speech.stop();
}

/// Opens Ajustes › Voz on its own (from the chat when the key is missing).
Future<void> openVoiceSettings([BuildContext? context]) async {
  final NavigatorState? nav = context != null ? Navigator.of(context) : AppNavigator.key.currentState;
  await nav?.push<void>(MaterialPageRoute<void>(
    settings: const RouteSettings(name: VoiceSettingsScreen.routeName),
    builder: (_) => const VoiceSettingsScreen(),
  ));
}

/// Registers dictation and read-aloud (OpenAI). Nothing is constructed
/// here: the registered objects are empty forwarders, and [VoiceServices]
/// is built on first use. Without a saved key the composer hides the mic
/// and "Leer en voz alta" leads to Ajustes › Voz.
void registerVoice() {
  VoiceServices.onNeedsKey = () => unawaited(openVoiceSettings());
  sl
    ..register<VoiceInput>(_LazyVoiceInput())
    ..register<Speaker>(_LazySpeaker());
}
