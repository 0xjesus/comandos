import 'dart:async';

import 'package:flutter/services.dart';

import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../core/polling/connectivity_watch.dart';
import '../chat/chat_controller.dart';
import '../voice/audio_out.dart';
import '../voice/dictation.dart';
import '../voice/speech.dart';
import '../voice/voice_services.dart';
import 'car_machine.dart';

/// [CarPorts] for the real app: the shared dictation engine, the speech
/// service, the outbox and the current session's [ChatController].
///
/// Text safety: Modo auto receives dictated text through a dictation sink
/// that stays open until the text is either durable in the outbox or saved
/// as a draft; only then may the recording be deleted. Leaving the screen
/// hands any such text to the chat composer (if open) or the draft.
class LiveCarPorts implements CarPorts {
  LiveCarPorts({
    required VoiceServices voice,
    required Outbox outbox,
    required ChatController? Function() chat,
    ConnectivityWatch? connectivity,
  })  : _v = voice,
        _outbox = outbox,
        _chat = chat,
        _connectivity = connectivity;

  final VoiceServices _v;
  final Outbox _outbox;
  final ChatController? Function() _chat;
  final ConnectivityWatch? _connectivity;

  String? _sinkOwner;
  Completer<void>? _release;
  String? _held;
  bool _disposed = false;

  DictationController get _d => _v.dictation;

  Future<void> _sink(String text) {
    if (_disposed) return _d.deliver(_sinkOwner ?? '', text, except: _sink);
    _held = text;
    return (_release = Completer<void>()).future;
  }

  void _attach(String owner) {
    _detach();
    _sinkOwner = owner;
    _d.attach(owner, _sink);
  }

  void _detach() {
    final String? o = _sinkOwner;
    if (o != null) _d.detach(o, _sink);
    _sinkOwner = null;
  }

  /// The text is safe elsewhere: let the dictation delete the audio.
  void _releaseHeld() {
    final Completer<void>? c = _release;
    _release = null;
    _held = null;
    _detach();
    if (c != null && !c.isCompleted) c.complete();
  }

  @override
  bool get continuous => _v.settings.value.continuous;

  Future<DictationOutcome> _watch(String owner, Future<DictationOutcome> Function() run, VoidCallback? onTranscribing) async {
    _attach(owner);
    void phase() {
      if (_d.activeOwner == owner && _d.phase == DictationPhase.transcribing) onTranscribing?.call();
    }

    _d.addListener(phase);
    try {
      final DictationOutcome o = await run();
      if (o is! DictationText) _detach();
      return o;
    } finally {
      _d.removeListener(phase);
    }
  }

  @override
  Future<DictationOutcome> listen(String session, {required VoidCallback onTranscribing}) =>
      _watch(session, () => _d.start(session), onTranscribing);

  @override
  void stopListening() => unawaited(_d.stop());

  @override
  void cancelListening() => unawaited(_d.cancel());

  @override
  Future<DictationOutcome> retry(String takeId) {
    final PendingTake? t = _d.take(takeId);
    if (t == null) return Future<DictationOutcome>.value(const DictationCancelled());
    return _watch(t.owner, () => _d.retry(takeId), null);
  }

  @override
  Future<void> discard(String takeId) => _d.discard(takeId);

  @override
  Future<bool> speak(String text) async => await _v.speech.say(text) == SpeakResult.completed;

  @override
  Future<bool> speakReply(String markdown) async =>
      await _v.speech.speakReply(markdown, mode: _v.settings.value.autoReadMode) == SpeakResult.completed;

  @override
  void stopSpeaking() => unawaited(_v.speech.stop());

  @override
  Future<SendOutcome> send(String session, String? pane, String text) async {
    final OutboxItem item;
    try {
      item = await _outbox.enqueue(
        session: session,
        pane: pane,
        text: text,
        transcriptBaseline: _chat()?.session == session ? _chat()?.baselineFor(text) : null,
      );
    } on OutboxRejection catch (e) {
      await saveDraft(session, text);
      return SendOutcome(SendKind.rejected, e.message);
    } catch (_) {
      await saveDraft(session, text);
      return const SendOutcome(SendKind.rejected, 'No pude guardar el mensaje.');
    }
    _releaseHeld(); // durable in the outbox now
    _chat()?.poke();
    return _delivery(item.id);
  }

  /// Waits a few seconds to tell "Enviado" from "se enviará al volver la red".
  Future<SendOutcome> _delivery(String id) async {
    if (_connectivity?.online.value == false) return const SendOutcome(SendKind.queuedOffline);
    final Completer<SendOutcome> done = Completer<SendOutcome>();
    void check() {
      if (done.isCompleted) return;
      final OutboxItem? i = _outbox.byId(id);
      if (i == null || i.state == OutboxState.delivered) {
        done.complete(const SendOutcome(SendKind.delivered));
      } else if (i.needsHuman) {
        done.complete(SendOutcome(SendKind.failed, outboxFailureMessage(i.failCode)));
      } else if (i.state == OutboxState.queued && i.lastError != null) {
        done.complete(SendOutcome(SendKind.queuedOffline, i.lastError!));
      }
    }

    final Timer t = Timer(const Duration(seconds: 8), () {
      if (!done.isCompleted) done.complete(const SendOutcome(SendKind.queuedOffline));
    });
    _outbox.addListener(check);
    check();
    try {
      return await done.future;
    } finally {
      _outbox.removeListener(check);
      t.cancel();
    }
  }

  @override
  Future<void> saveDraft(String session, String text) async {
    final bool held = _release != null;
    final Completer<void>? c = _release;
    _release = null;
    _held = null;
    final TextSink self = _sink;
    _detach();
    try {
      // To the chat composer if it is open underneath, else the draft file.
      await _d.deliver(session, text, except: self);
    } finally {
      if (held && c != null && !c.isCompleted) c.complete();
    }
  }

  @override
  Future<String?> answer(String session, String optionKey) async {
    final ChatController? c = _chat();
    if (c == null || c.session != session) return 'La sesión cambió; vuelve a intentarlo.';
    return c.answer(optionKey);
  }

  @override
  Future<String?> interrupt(String session) async {
    final ChatController? c = _chat();
    if (c == null || c.session != session) return 'La sesión cambió; vuelve a intentarlo.';
    return c.interrupt();
  }

  @override
  void markRead(String session, String replyId) => unawaited(_v.settings.markRead(session, replyId));

  @override
  Future<void> cue() async {
    unawaited(HapticFeedback.mediumImpact());
    try {
      await _v.speech.playFile(await earconFile(await _v.cacheDir()));
    } catch (_) {}
  }

  /// Leaving Modo auto: text held but not yet sent goes to the composer or
  /// the draft; anything still being transcribed lands there on its own.
  void dispose() {
    _disposed = true;
    final String? owner = _sinkOwner;
    final String? text = _held;
    if (owner != null && text != null && _release != null) {
      unawaited(saveDraft(owner, text));
    } else {
      _detach();
    }
  }
}
