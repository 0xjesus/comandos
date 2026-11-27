import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import 'voice_settings.dart';

/// The only place the OpenAI key is ever sent. Not configurable on purpose.
const String kOpenAiBase = 'https://api.openai.com/v1';

/// Literal technical terms the dictation should not mangle.
const List<String> kSttKeywords = <String>[
  'git', 'commit', 'push', 'pull request', 'merge', 'rebase', 'branch', 'deploy', //
  'Claude', 'Codex', 'tmux', 'Flutter', 'API', 'ComandOS', 'Tailscale', 'npm', 'Rust', 'cargo',
];

enum OpenAiErrorKind {
  missingKey,
  unauthorized,
  forbidden,
  quota,
  rateLimited,
  badRequest,
  notFound,
  tooLarge,
  server,
  network,
  timeout,
  aborted,
  badResponse,
}

/// A failed OpenAI call. Carries no server text and never the key: OpenAI's
/// own 401 message echoes part of the key, so it is deliberately dropped.
class OpenAiException implements Exception {
  const OpenAiException(this.kind, {this.status});

  final OpenAiErrorKind kind;
  final int? status;

  /// Worth retrying automatically (the network blinked, OpenAI hiccupped).
  bool get retryable =>
      kind == OpenAiErrorKind.network ||
      kind == OpenAiErrorKind.timeout ||
      kind == OpenAiErrorKind.server ||
      kind == OpenAiErrorKind.rateLimited;

  String get userMessage {
    switch (kind) {
      case OpenAiErrorKind.missingKey:
        return 'Falta la llave de OpenAI. Agrégala en Ajustes › Voz.';
      case OpenAiErrorKind.unauthorized:
        return 'OpenAI rechazó la llave. Revísala en Ajustes › Voz.';
      case OpenAiErrorKind.forbidden:
        return 'OpenAI no permite usar esa llave para esto (403).';
      case OpenAiErrorKind.quota:
        return 'Tu cuenta de OpenAI no tiene saldo o llegó a su límite de gasto.';
      case OpenAiErrorKind.rateLimited:
        return 'OpenAI pide esperar un momento (demasiadas solicitudes).';
      case OpenAiErrorKind.badRequest:
        return 'OpenAI no aceptó la solicitud. Prueba otro modelo en Ajustes › Voz.';
      case OpenAiErrorKind.notFound:
        return 'Tu cuenta de OpenAI no tiene acceso a ese modelo.';
      case OpenAiErrorKind.tooLarge:
        return 'La grabación es demasiado grande para OpenAI.';
      case OpenAiErrorKind.server:
        return 'OpenAI tuvo un problema. Inténtalo de nuevo.';
      case OpenAiErrorKind.network:
        return 'Sin conexión con OpenAI.';
      case OpenAiErrorKind.timeout:
        return 'OpenAI tardó demasiado en responder.';
      case OpenAiErrorKind.aborted:
        return 'Cancelado.';
      case OpenAiErrorKind.badResponse:
        return 'OpenAI respondió algo inesperado.';
    }
  }

  @override
  String toString() => 'OpenAiException(${kind.name}${status == null ? '' : ', $status'})';
}

/// Maps an HTTP status (and, for 429, the error code in the body) to a kind.
OpenAiException mapOpenAiStatus(int status, List<int> body) {
  switch (status) {
    case 401:
      return OpenAiException(OpenAiErrorKind.unauthorized, status: status);
    case 403:
      return OpenAiException(OpenAiErrorKind.forbidden, status: status);
    case 404:
      return OpenAiException(OpenAiErrorKind.notFound, status: status);
    case 413:
      return OpenAiException(OpenAiErrorKind.tooLarge, status: status);
    case 429:
      String code = '';
      try {
        final Object? j = jsonDecode(utf8.decode(body, allowMalformed: true));
        if (j is Map && j['error'] is Map) {
          final Map<Object?, Object?> e = j['error'] as Map<Object?, Object?>;
          code = '${e['code'] ?? ''} ${e['type'] ?? ''}';
        }
      } catch (_) {}
      return OpenAiException(
          code.contains('insufficient_quota') ? OpenAiErrorKind.quota : OpenAiErrorKind.rateLimited,
          status: status);
  }
  if (status == 400 || status == 415 || status == 422) {
    return OpenAiException(OpenAiErrorKind.badRequest, status: status);
  }
  if (status >= 500) return OpenAiException(OpenAiErrorKind.server, status: status);
  return OpenAiException(OpenAiErrorKind.badResponse, status: status);
}

/// What the speech-to-text request needs besides the audio.
class SttOptions {
  const SttOptions({required this.model, required this.language});

  final String model;
  final SttLanguage language;

  /// `gpt-transcribe` takes `keywords[]` and `languages[]`; older models a
  /// single `language` and the vocabulary inside `prompt` (docs, 2026-11).
  bool get modernFields => model == 'gpt-transcribe' || model.startsWith('gpt-transcribe-');

  String get prompt {
    final String terms = kSttKeywords.join(', ');
    switch (language) {
      case SttLanguage.es:
        return modernFields
            ? 'Dictado en español para un agente de programación.'
            : 'Dictado en español para un agente de programación. Términos: $terms.';
      case SttLanguage.en:
        return modernFields
            ? 'Dictation for a coding agent.'
            : 'Dictation for a coding agent. Terms: $terms.';
      case SttLanguage.auto:
        return modernFields ? '' : terms;
    }
  }
}

/// What the text-to-speech request needs besides the text.
class TtsOptions {
  const TtsOptions({required this.model, required this.voice});

  final String model;
  final String voice;
  static const String format = 'mp3';

  /// `instructions` is accepted by gpt-4o-mini-tts, not by tts-1 / tts-1-hd.
  String? get instructions => model.startsWith('tts-1')
      ? null
      : 'Lee en español latinoamericano, con tono natural, claro y tranquilo. '
          'Lee los términos técnicos en inglés tal cual.';

  /// Identity of the synthesised audio (cache key ingredient).
  String get signature => '$model|$voice|${instructions ?? ''}|$format';
}

/// Minimal OpenAI client: transcription, speech, key check. One keep-alive
/// HTTP client, created on first use. The key comes from [key] per call and
/// only ever goes to [kOpenAiBase].
class OpenAiClient {
  OpenAiClient({
    required Future<String?> Function() key,
    http.Client Function()? clientFactory,
    this.sttTimeout = const Duration(seconds: 60),
    this.ttsTimeout = const Duration(seconds: 30),
    this.checkTimeout = const Duration(seconds: 15),
  })  : _key = key,
        _clientFactory = clientFactory ?? http.Client.new;

  final Future<String?> Function() _key;
  final http.Client Function() _clientFactory;
  final Duration sttTimeout;
  final Duration ttsTimeout;
  final Duration checkTimeout;
  http.Client? _client;

  http.Client get _http => _client ??= _clientFactory();

  /// `POST /v1/audio/transcriptions` (multipart). Returns the text (may be
  /// empty when nothing was said). Throws [OpenAiException].
  Future<String> transcribe(String path, SttOptions o, {Future<void>? abort}) async {
    final String key = await _requireKey();
    final Uint8List audio;
    try {
      audio = await File(path).readAsBytes();
    } catch (_) {
      throw const OpenAiException(OpenAiErrorKind.badRequest);
    }
    // Modo auto's takes are WAV (recorded natively); dictation is AAC.
    final bool wav = path.endsWith('.wav');
    final MultipartBuilder body = MultipartBuilder()
      ..field('model', o.model)
      ..file('file', wav ? 'dictado.wav' : 'dictado.m4a', wav ? 'audio/wav' : 'audio/mp4', audio);
    final String prompt = o.prompt;
    if (prompt.isNotEmpty) body.field('prompt', prompt);
    if (o.modernFields) {
      for (final String k in kSttKeywords) {
        body.field('keywords[]', k);
      }
      if (o.language != SttLanguage.auto) body.field('languages[]', o.language.name);
    } else if (o.language != SttLanguage.auto) {
      body.field('language', o.language.name);
    }
    final http.Response r = await _send(
      'POST',
      '/audio/transcriptions',
      key: key,
      contentType: body.contentType,
      body: body.finish(),
      // A long dictation takes longer to upload and transcribe: one more
      // second per second of audio (AAC 32 kbps ≈ 4 KB/s, WAV 16 kHz 16-bit
      // = 32 KB/s), on top of the base.
      timeout: sttTimeout + Duration(seconds: audio.length ~/ (wav ? 32000 : 4000)),
      abort: abort,
    );
    try {
      final Object? j = jsonDecode(utf8.decode(r.bodyBytes));
      if (j is Map && j['text'] is String) return j['text'] as String;
    } catch (_) {}
    throw const OpenAiException(OpenAiErrorKind.badResponse);
  }

  /// `POST /v1/audio/speech`. Returns the encoded audio ([TtsOptions.format]).
  Future<Uint8List> speech(String text, TtsOptions o, {Future<void>? abort}) async {
    final String key = await _requireKey();
    final Map<String, Object?> body = <String, Object?>{
      'model': o.model,
      'input': text.length > 4096 ? text.substring(0, 4096) : text,
      'voice': o.voice,
      'response_format': TtsOptions.format,
      if (o.instructions != null) 'instructions': o.instructions,
    };
    final http.Response r = await _send(
      'POST',
      '/audio/speech',
      key: key,
      contentType: 'application/json',
      body: utf8.encode(jsonEncode(body)),
      timeout: ttsTimeout,
      abort: abort,
    );
    if (r.bodyBytes.isEmpty) throw const OpenAiException(OpenAiErrorKind.badResponse);
    return r.bodyBytes;
  }

  /// Proves a key works: `GET /v1/models/{model}` (free). Uses [candidate]
  /// when given (a key not saved yet), else the stored one.
  Future<void> check(String model, {String? candidate}) async {
    final String key = candidate ?? await _requireKey();
    await _send('GET', '/models/$model', key: key, timeout: checkTimeout);
  }

  void close() {
    _client?.close();
    _client = null;
  }

  Future<String> _requireKey() async {
    final String? k = await _key();
    if (k == null || k.isEmpty) throw const OpenAiException(OpenAiErrorKind.missingKey);
    return k;
  }

  Future<http.Response> _send(
    String method,
    String path, {
    required String key,
    String? contentType,
    List<int>? body,
    required Duration timeout,
    Future<void>? abort,
  }) async {
    final Completer<void> stop = Completer<void>();
    bool timedOut = false;
    bool aborted = false;
    final Timer t = Timer(timeout, () {
      timedOut = true;
      if (!stop.isCompleted) stop.complete();
    });
    abort?.then((_) {
      aborted = true;
      if (!stop.isCompleted) stop.complete();
    });
    final http.AbortableRequest req = http.AbortableRequest(method, Uri.parse('$kOpenAiBase$path'),
        abortTrigger: stop.future)
      ..headers['Authorization'] = 'Bearer $key'
      ..headers['Accept'] = '*/*';
    if (contentType != null) req.headers['Content-Type'] = contentType;
    if (body != null) req.bodyBytes = body;
    try {
      final http.StreamedResponse s = await _http.send(req);
      final http.Response r = await http.Response.fromStream(s);
      if (r.statusCode >= 200 && r.statusCode < 300) return r;
      throw mapOpenAiStatus(r.statusCode, r.bodyBytes);
    } on OpenAiException {
      rethrow;
    } on http.RequestAbortedException {
      throw OpenAiException(timedOut && !aborted ? OpenAiErrorKind.timeout : OpenAiErrorKind.aborted);
    } on TimeoutException {
      throw const OpenAiException(OpenAiErrorKind.timeout);
    } on SocketException {
      throw const OpenAiException(OpenAiErrorKind.network);
    } on HandshakeException {
      throw const OpenAiException(OpenAiErrorKind.network);
    } on http.ClientException {
      throw OpenAiException(aborted
          ? OpenAiErrorKind.aborted
          : timedOut
              ? OpenAiErrorKind.timeout
              : OpenAiErrorKind.network);
    } on HttpException {
      throw const OpenAiException(OpenAiErrorKind.network);
    } finally {
      t.cancel();
    }
  }
}

/// A `multipart/form-data` body that, unlike `http.MultipartRequest`, can
/// repeat a field name (`keywords[]`, `languages[]`). Plain fields carry no
/// Content-Type, exactly like `curl -F`.
class MultipartBuilder {
  MultipartBuilder({String? boundary}) : boundary = boundary ?? _randomBoundary();

  final String boundary;
  final BytesBuilder _b = BytesBuilder(copy: false);

  String get contentType => 'multipart/form-data; boundary=$boundary';

  static String _randomBoundary() {
    const String chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    final Random r = Random.secure();
    return 'comandos-${List<String>.generate(24, (_) => chars[r.nextInt(chars.length)]).join()}';
  }

  static String _q(String s) => s.replaceAll('"', '%22').replaceAll('\r', ' ').replaceAll('\n', ' ');

  void field(String name, String value) {
    _b.add(utf8.encode('--$boundary\r\ncontent-disposition: form-data; name="${_q(name)}"\r\n\r\n'));
    _b.add(utf8.encode(value));
    _b.add(utf8.encode('\r\n'));
  }

  void file(String name, String filename, String contentType, List<int> bytes) {
    _b.add(utf8.encode('--$boundary\r\ncontent-disposition: form-data; name="${_q(name)}"; '
        'filename="${_q(filename)}"\r\ncontent-type: $contentType\r\n\r\n'));
    _b.add(bytes);
    _b.add(utf8.encode('\r\n'));
  }

  Uint8List finish() {
    _b.add(utf8.encode('--$boundary--\r\n'));
    return _b.takeBytes();
  }
}
