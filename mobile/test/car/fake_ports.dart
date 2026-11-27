import 'dart:async';

import 'package:comandos_mobile/features/car/car_mic.dart';
import 'package:comandos_mobile/features/car/car_ports.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:flutter/foundation.dart';

/// [CarPorts] for tests: records what Modo auto says, sends and hears.
class FakePorts implements CarPorts {
  @override
  bool continuous = false;
  bool autoSpeak = true;

  /// What [speak] / [speakReply] report when [autoSpeak] (false = TTS failed).
  bool speakOk = true;
  final List<bool> requireSpeech = <bool>[];
  final List<(String?, String)> answerBindings = <(String?, String)>[];
  SendOutcome sendResult = const SendOutcome(SendKind.delivered);
  String? answerError;

  final List<String> log = <String>[];
  final List<String> spoken = <String>[];
  final List<(String, String?, String)> sent = <(String, String?, String)>[];
  final List<(String, String)> drafts = <(String, String)>[];
  final List<(String, String)> answers = <(String, String)>[];
  final List<String> reads = <String>[];
  final List<String> retried = <String>[];
  final List<String> discarded = <String>[];
  Completer<DictationOutcome>? listening;
  VoidCallback? onTranscribing;
  Completer<bool>? speaking;
  int cues = 0;

  /// What the person said in the current dictation.
  void dictated(DictationOutcome o) {
    final Completer<DictationOutcome> c = listening!;
    listening = null;
    c.complete(o);
  }

  final List<bool> shortAnswers = <bool>[];

  /// What «OK ComandOS» matched each time the call started a turn from it.
  final List<String> wakeTakes = <String>[];

  @override
  void takeAfterWake(String heard) {
    log.add('takeAfterWake');
    wakeTakes.add(heard);
  }

  @override
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false, bool shortAnswer = false}) {
    log.add('listen:$session');
    this.requireSpeech.add(requireSpeech);
    shortAnswers.add(shortAnswer);
    this.onTranscribing = onTranscribing;
    return (listening = Completer<DictationOutcome>()).future;
  }

  /// A command listen in progress ([hear]); null when not listening.
  Completer<String?>? hearing;
  final List<Duration> hearWindows = <Duration>[];

  /// Nothing can listen on the phone ([hear] returns null).
  bool canHear = true;

  @override
  Future<String?> hear(Duration window, {bool Function(String partial)? until}) {
    hearWindows.add(window);
    log.add('hear');
    if (!canHear) return Future<String?>.value(null);
    return (hearing = Completer<String?>()).future;
  }

  /// The person says [text] while a command listen is open.
  void command(String text) {
    final Completer<String?> c = hearing!;
    hearing = null;
    c.complete(text);
  }

  @override
  void stopHearing() {
    log.add('stopHearing');
    final Completer<String?>? c = hearing;
    hearing = null;
    if (c != null && !c.isCompleted) c.complete('');
  }

  @override
  void stopListening() {
    log.add('stopListening');
    onTranscribing?.call();
  }

  @override
  void cancelListening() {
    log.add('cancelListening');
    if (listening != null) dictated(const DictationCancelled());
  }

  @override
  Future<DictationOutcome> retry(String takeId) {
    retried.add(takeId);
    return (listening = Completer<DictationOutcome>()).future;
  }

  @override
  Future<void> discard(String takeId) async => discarded.add(takeId);

  @override
  void releaseDictation() => log.add('release');

  Future<bool> _talk(String what) {
    spoken.add(what);
    if (autoSpeak) return Future<bool>.value(speakOk);
    return (speaking = Completer<bool>()).future;
  }

  @override
  Future<bool> speak(String text) => _talk(text);

  @override
  Future<bool> speakReply(String markdown) => _talk('REPLY:$markdown');

  /// Ends the current utterance naturally.
  void spoke() {
    final Completer<bool>? c = speaking;
    speaking = null;
    c?.complete(true);
  }

  @override
  void stopSpeaking() {
    log.add('stopSpeaking');
    final Completer<bool>? c = speaking;
    speaking = null;
    if (c != null && !c.isCompleted) c.complete(false);
  }

  @override
  Future<SendOutcome> send(String session, String? pane, String text) async {
    sent.add((session, pane, text));
    return sendResult;
  }

  @override
  Future<void> saveDraft(String session, String text) async => drafts.add((session, text));

  @override
  Future<String?> answer(String session, String optionKey, {String? pane, required String promptSig}) async {
    answers.add((session, optionKey));
    answerBindings.add((pane, promptSig));
    return answerError;
  }

  @override
  Future<String?> interrupt(String session) async {
    log.add('interrupt:$session');
    return null;
  }

  @override
  void markRead(String session, String replyId) => reads.add('$session:$replyId');

  @override
  Future<void> cue() async => cues++;
}

/// Modo auto's microphone for tests: each take waits for [done] (or
/// completes at once with [next]).
class FakeCarMic implements CarMic {
  final ValueNotifier<double> _level = ValueNotifier<double>(0);
  final List<String> log = <String>[];

  /// The wake text each take started after (null: a follow-up).
  final List<String?> takes = <String?>[];
  final List<Duration> silences = <Duration>[];
  Completer<MicTake>? pending;

  /// The next take ends at once with this.
  MicTake? next;

  @override
  ValueListenable<double> get level => _level;

  @override
  Future<MicTake> take({required Duration silence, String? afterWake}) {
    log.add('take');
    takes.add(afterWake);
    silences.add(silence);
    final MicTake? n = next;
    next = null;
    if (n != null) return Future<MicTake>.value(n);
    return (pending = Completer<MicTake>()).future;
  }

  /// The take in progress ends with [t].
  void done(MicTake t) {
    final Completer<MicTake> c = pending!;
    pending = null;
    c.complete(t);
  }

  @override
  Future<void> end() async => log.add('end');

  @override
  Future<void> cancel() async {
    log.add('cancel');
    final Completer<MicTake>? c = pending;
    pending = null;
    if (c != null && !c.isCompleted) c.complete(const MicTake(end: 'cancelled'));
  }

  @override
  Future<void> release() async => log.add('release');
}
