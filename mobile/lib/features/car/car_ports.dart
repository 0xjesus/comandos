import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../../core/models/ask_prompt.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../core/polling/connectivity_watch.dart';
import '../chat/chat_controller.dart';
import '../voice/audio_out.dart';
import '../voice/dictation.dart';
import '../voice/text_prep.dart';
import '../voice/voice_services.dart';
import '../voice/voice_settings.dart';
import 'car_mic.dart';
import 'edge_voice.dart';
import 'on_device.dart';
import 'wake_phrase.dart';

enum SendKind {
  delivered,
  queuedOffline,
  failed,
  rejected,

  /// An earlier message of that session is failed / uncertain: this one
  /// waits behind it until the person acts in the chat.
  blocked,

  /// The desktop refused because a permission prompt is open; the outbox
  /// delivers it once the prompt is answered.
  waitingPrompt,
}

class SendOutcome {
  const SendOutcome(this.kind, [this.message = '']);
  final SendKind kind;
  final String message;
}

/// Everything Modo auto does to the outside world. The live version wires
/// dictation, speech, the outbox and the chat; tests use a fake.
abstract class CarPorts {
  /// Conversación continua.
  bool get continuous;

  /// «OK ComandOS» matched in [heard] and the listener still has the
  /// microphone: the person's turn starts now (before the earcon), so words
  /// said in the same breath are kept. The next [listen] takes it.
  void takeAfterWake(String heard);

  /// Records (auto-stops on silence) and transcribes for [session].
  /// [onTranscribing] fires when recording ends and transcription starts.
  /// [requireSpeech]: a recording in which no speech was detected is
  /// dropped without transcribing ([DictationNothing]).
  /// [shortAnswer]: an answer to a permission prompt ("dos", "sí"): the
  /// phone's own recognizer, never OpenAI.
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false, bool shortAnswer = false});
  void stopListening();
  void cancelListening();

  /// Listens once, with the phone's own recognizer (free), for a spoken
  /// command: '' when nothing was said, null when nothing can listen.
  /// [until] may end it early on a partial result.
  Future<String?> hear(Duration window, {bool Function(String partial)? until});

  /// Ends [hear] now (it completes with '').
  void stopHearing();
  Future<DictationOutcome> retry(String takeId);
  Future<void> discard(String takeId);

  /// The last dictated text was consumed as a spoken answer (or a failed
  /// attempt at one): let its recording go, without keeping it as a draft.
  void releaseDictation();

  /// Speaks plain text. True only when it played to the end.
  Future<bool> speak(String text);

  /// Reads a reply (markdown) with the configured read mode.
  Future<bool> speakReply(String markdown);
  void stopSpeaking();

  /// Through the outbox (durable); reports how delivery went.
  Future<SendOutcome> send(String session, String? pane, String text);

  /// Keeps dictated text that will not be sent (as the session's draft).
  Future<void> saveDraft(String session, String text);

  /// Answers the prompt whose [promptSig] the choice was made for, in
  /// [pane]. Returns an error (and sends nothing) if the chat now shows
  /// another session, pane or prompt.
  Future<String?> answer(String session, String optionKey, {String? pane, required String promptSig});
  Future<String?> interrupt(String session);
  void markRead(String session, String replyId);

  /// Earcon + haptic: "your turn". Completes when the earcon ended.
  Future<void> cue();
}

/// Buttons on the steering wheel or a headset (media keys).
enum CarButton {
  talk,
  next,
  previous;

  static CarButton? named(String name) => switch (name) {
        'talk' => CarButton.talk,
        'next' => CarButton.next,
        'previous' => CarButton.previous,
        _ => null,
      };
}

/// [CarPorts] for the real app: the phone's own voice and recognizer, the
/// shared dictation engine (OpenAI), the outbox and the current split's
/// [ChatController].
///
/// Voice: Modo auto speaks with Edge (free, rendered by the desktop) or the
/// phone's voice. What the person says is recorded by Modo auto's own
/// microphone ([CarMic]: the listener that heard «OK ComandOS» keeps going,
/// and the turn ends when no new words are heard) and transcribed like the
/// chat's dictation (OpenAI with the person's key); without a key, or when
/// OpenAI fails, what the offline recognizer heard is the text. A take in
/// which words were heard is never dropped. Short spoken answers use the
/// phone's recognizer. Without [CarMic] (tests, no model) the chat's
/// dictation records instead.
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
    DeviceSpeech? tts,
    DeviceListener? stt,
    CarMic? mic,
    Future<void> Function()? holdFocus,
    Future<void> Function()? releaseFocus,
  })  : _v = voice,
        _outbox = outbox,
        _chat = chat,
        _connectivity = connectivity,
        _tts = tts ?? DeviceVoice.tts,
        _stt = stt ?? DeviceVoice.stt,
        _mic = mic,
        _holdFocus = holdFocus ?? holdFocusForRecording,
        _releaseFocus = releaseFocus ?? releaseAudioFocus;

  final VoiceServices _v;
  final Outbox _outbox;
  final ChatController? Function() _chat;
  final ConnectivityWatch? _connectivity;
  final DeviceSpeech _tts;
  final DeviceListener _stt;
  final CarMic? _mic;
  final Future<void> Function() _holdFocus;
  final Future<void> Function() _releaseFocus;

  /// The take «OK ComandOS» started (the microphone never let go), waiting
  /// for [listen].
  Future<MicTake>? _armed;

  /// A [CarMic] take is recording.
  bool _taking = false;

  /// Input level 0..1 of the take recording now (null: no [CarMic]).
  ValueListenable<double>? get micLevel => _mic?.level;

  /// The phone's recognizer is taking the current dictation (not OpenAI).
  bool _deviceListening = false;
  bool _deviceCancelled = false;
  int _speakGen = 0;

  VoiceSettings get _settings => _v.settings.value;

  Duration get _silence => Duration(milliseconds: (_settings.silenceSeconds * 1000).round());

  @override
  void takeAfterWake(String heard) {
    final CarMic? mic = _mic;
    if (mic == null) return;
    _armed = mic.take(silence: _silence, afterWake: heard);
  }



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
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false, bool shortAnswer = false}) async {
    // A text still held here was neither sent nor released: keep it as a
    // draft before a new dictation takes over (its recording is then let
    // go, never orphaned).
    final String? owner = _sinkOwner;
    final String? held = _held;
    if (owner != null && held != null && _release != null) await saveDraft(owner, held);
    final CarMic? mic = _mic;
    if (!shortAnswer && mic != null) {
      final Future<MicTake> take = _armed ?? mic.take(silence: _silence);
      _armed = null;
      final DictationOutcome? o = await _fromTake(session, take, onTranscribing);
      if (o != null) return o;
      // Modo auto's microphone could not record: the chat's dictation does.
    }
    // What the person says is transcribed like the chat's dictation (the
    // same recording, silence detection and OpenAI transcription, which
    // understands whole sentences); the phone's recognizer cut sentences
    // short and misheard them. Without a key, or for a short answer, the
    // phone's recognizer.
    await _v.keys.load();
    if (!shortAnswer && _v.keys.status.value.present) {
      return _watch(session, () => _d.start(session, requireSpeech: requireSpeech), onTranscribing);
    }
    final DictationOutcome? o = await _listenOnDevice(session, shortAnswer: shortAnswer);
    return o ?? DictationDenied(_stt.problem ?? 'Este teléfono no tiene reconocimiento de voz.');
  }

  /// The person's turn from a [CarMic] take: transcribed with OpenAI (key
  /// present), else, or when that fails, what the offline recognizer heard.
  /// Null when nothing could be recorded.
  Future<DictationOutcome?> _fromTake(String session, Future<MicTake> pending, VoidCallback onTranscribing) async {
    _taking = true;
    unawaited(_holdFocus()); // music pauses while the person talks
    final MicTake t;
    try {
      t = await pending;
    } finally {
      _taking = false;
      unawaited(_releaseFocus());
    }
    final String? path = t.path;
    if (t.unusable) return null;
    if (t.cancelled || path == null) {
      if (path != null) await _rm(path);
      return t.cancelled ? const DictationCancelled() : const DictationNothing();
    }
    if (!t.words) {
      // Silence or road noise: never turned into a message.
      await _rm(path);
      return const DictationNothing();
    }
    final String offline = stripWakePhrase(t.text);
    onTranscribing();
    await _v.keys.load();
    if (_v.keys.status.value.present) {
      final DictationOutcome o = await _watch(session, () => _d.adopt(session, path), null);
      switch (o) {
        case DictationText(:final String text):
          final String said = stripWakePhrase(text);
          if (said.isNotEmpty) {
            if (_held != null) _held = said; // a draft keeps it without the phrase
            return DictationText(said);
          }
          _releaseHeld(); // only the wake phrase: the offline text decides
        case DictationCancelled():
          return o;
        case DictationFailed(:final PendingTake take):
          // OpenAI failed after its retries: the offline text goes instead.
          await _d.discard(take.id);
        case DictationNothing():
          break;
        case DictationDenied():
          await _rm(path);
      }
    } else {
      await _rm(path);
    }
    if (offline.isEmpty) return const DictationNothing();
    return _hold(session, offline);
  }

  static Future<void> _rm(String path) async {
    try {
      await File(path).delete();
    } catch (_) {}
  }

  /// [text] (not from the dictation) is held like a transcription until
  /// sent, saved or released: leaving the screen keeps it as a draft.
  Future<DictationOutcome> _hold(String session, String text) async {
    if (_disposed) {
      await _d.deliver(session, text);
      return const DictationCancelled();
    }
    _detach();
    _sinkOwner = session;
    _held = text;
    _release = Completer<void>();
    return DictationText(text);
  }

  /// One dictation with the phone's recognizer; null when it cannot
  /// listen. The text is held like a transcription until sent, saved or
  /// released, so leaving the screen keeps it as a draft.
  Future<DictationOutcome?> _listenOnDevice(String session, {required bool shortAnswer}) async {
    await _mic?.release(); // the phone's recognizer needs the microphone
    _deviceListening = true;
    _deviceCancelled = false;
    final String? t;
    try {
      t = await _stt.listen(
        listenFor: Duration(seconds: shortAnswer ? 10 : 60),
        pauseFor: Duration(milliseconds: ((shortAnswer ? 2.0 : _settings.silenceSeconds + 1.0) * 1000).round()),
        dictation: !shortAnswer,
        language: _settings.language,
      );
    } finally {
      _deviceListening = false;
    }
    if (t == null) return null;
    if (_deviceCancelled) return const DictationCancelled();
    final String text = t.trim();
    if (text.isEmpty) return const DictationNothing();
    return _hold(session, text);
  }

  @override
  void releaseDictation() => _releaseHeld();

  @override
  void stopListening() {
    final CarMic? mic = _mic;
    if (mic != null && (_taking || _armed != null)) {
      unawaited(mic.end()); // the turn ends now and is transcribed
      return;
    }
    unawaited(_deviceListening ? _stt.stop() : _d.stop());
  }

  @override
  void cancelListening() {
    final CarMic? mic = _mic;
    if (mic != null && (_taking || _armed != null)) unawaited(mic.cancel());
    _armed = null;
    if (_deviceListening) {
      _deviceCancelled = true;
      unawaited(_stt.cancel());
    } else {
      unawaited(_d.cancel());
    }
  }

  @override
  Future<String?> hear(Duration window, {bool Function(String partial)? until}) async {
    await _mic?.release(); // the phone's recognizer needs the microphone
    return _stt.listen(
      listenFor: window,
      pauseFor: window,
      dictation: false,
      language: _settings.language,
      until: until,
    );
  }

  @override
  void stopHearing() => unawaited(_stt.cancel());

  @override
  Future<DictationOutcome> retry(String takeId) {
    final PendingTake? t = _d.take(takeId);
    if (t == null) return Future<DictationOutcome>.value(const DictationCancelled());
    return _watch(t.owner, () => _d.retry(takeId), null);
  }

  @override
  Future<void> discard(String takeId) => _d.discard(takeId);

  @override
  Future<bool> speak(String text) {
    _speakGen++;
    return _tts.speak(text, language: _settings.language, speed: _settings.speed);
  }

  /// Reads a reply (markdown → speakable text) WHOLE, chunk by chunk: the
  /// orchestrator talks to be heard, so the chat's read mode (which may
  /// read only the ending) does not apply. With Edge the next chunk renders
  /// while this one plays. [stopSpeaking] («Hablar», the steering wheel)
  /// cuts it between or during chunks.
  @override
  Future<bool> speakReply(String markdown) async {
    final int gen = ++_speakGen;
    final List<String> chunks = carReplyChunks(markdown);
    final DeviceSpeech tts = _tts;
    for (int i = 0; i < chunks.length; i++) {
      if (tts is EdgeVoice && i + 1 < chunks.length) tts.prepare(chunks[i + 1], speed: _settings.speed);
      if (!await tts.speak(chunks[i], language: _settings.language, speed: _settings.speed)) return false;
      if (gen != _speakGen) return false;
    }
    return gen == _speakGen;
  }

  @override
  void stopSpeaking() {
    _speakGen++;
    unawaited(_tts.stop());
  }

  @override
  Future<SendOutcome> send(String session, String? pane, String text) async {
    final OutboxItem item;
    // The pane whose replies are being read: the chat's, when it is open
    // for this session (the feed's pane may be another one).
    final ChatController? chat = _chat()?.session == session ? _chat() : null;
    try {
      item = await _outbox.enqueue(
        session: session,
        pane: chat?.pane ?? pane,
        text: text,
        transcriptBaseline: chat?.baselineFor(text),
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

  /// An earlier message of the item's session that waits for the person
  /// (failed, uncertain, being edited): the item cannot go out on its own.
  bool _blockedByEarlier(OutboxItem item) {
    for (final OutboxItem i in _outbox.itemsFor(item.session)) {
      if (i.id == item.id) return false;
      if (i.needsHuman) return true;
    }
    return false;
  }

  /// Waits a few seconds to tell "Enviado" from "se enviará al volver la
  /// red", and says so when the message is stuck behind an earlier one.
  Future<SendOutcome> _delivery(String id) async {
    final OutboxItem? first = _outbox.byId(id);
    if (first != null && _blockedByEarlier(first)) return const SendOutcome(SendKind.blocked);
    if (_connectivity?.online.value == false) return const SendOutcome(SendKind.queuedOffline);
    final Completer<SendOutcome> done = Completer<SendOutcome>();
    void check() {
      if (done.isCompleted) return;
      final OutboxItem? i = _outbox.byId(id);
      if (i == null || i.state == OutboxState.delivered) {
        done.complete(const SendOutcome(SendKind.delivered));
      } else if (i.needsHuman) {
        done.complete(SendOutcome(SendKind.failed, outboxFailureMessage(i.failCode)));
      } else if (_blockedByEarlier(i)) {
        done.complete(const SendOutcome(SendKind.blocked));
      } else if (i.waitCode == WaitCode.promptOpen) {
        done.complete(const SendOutcome(SendKind.waitingPrompt));
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
  Future<String?> answer(String session, String optionKey, {String? pane, required String promptSig}) async {
    final ChatController? c = _chat();
    if (c == null || c.session != session) return 'La sesión cambió; no respondí.';
    final AskPrompt? shown = c.ask;
    if (shown == null || shown.signature != promptSig || (pane != null && c.pane != pane)) {
      return 'La pregunta cambió; no respondí.';
    }
    final AskOption? option = shown.options.where((AskOption o) => o.key == optionKey).firstOrNull;
    if (option == null) return 'Esa opción ya no está; no respondí.';
    // Reads the pane again on the server: only the very prompt that was read
    // aloud is answered, never one that replaced it a second ago.
    return c.answerPrompt(shown, option);
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
    _speakGen++;
    final CarMic? mic = _mic;
    if (mic != null && (_taking || _armed != null)) unawaited(mic.cancel());
    _armed = null;
    if (_deviceListening) unawaited(_stt.stop()); // its text lands in the draft
    final String? owner = _sinkOwner;
    final String? text = _held;
    if (owner != null && text != null && _release != null) {
      unawaited(saveDraft(owner, text));
    } else {
      _detach();
    }
  }
}

/// A reply read in Modo auto, all of it: a short first chunk (it starts
/// fast), then chunks short enough for Edge to render while the previous
/// one plays.
List<String> carReplyChunks(String markdown) =>
    chunkForSpeech(selectForReading(prepareForSpeech(markdown), ReadMode.full), rest: 600);
