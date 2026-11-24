export 'api_response.dart';

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';

import '../config/paired_server.dart';
import '../models/session_feed.dart';
import '../models/transcript.dart';
import 'api_errors.dart';
import 'api_response.dart';
import 'json_decode.dart';

/// Per-request time budgets.
class ApiTimeouts {
  static const Duration connect = Duration(seconds: 6);
  static const Duration get = Duration(seconds: 10);
  static const Duration send = Duration(seconds: 20);
  static const Duration longPoll = Duration(seconds: 35);
}

/// The HTTP client for one paired desktop.
///
/// * one shared keep-alive connection pool;
/// * `X-Comandos-Token` on every request (never logged);
/// * every request has a hard deadline and is really aborted when it passes;
/// * HTTP statuses never throw from [request]; transport failures throw
///   [NetworkException] / [RequestTimeoutException].
class ComandosApi {
  ComandosApi(this.server, {http.Client? client})
      : _client = client ?? IOClient(_newHttpClient());

  final PairedServer server;
  final http.Client _client;
  bool _closed = false;

  static HttpClient _newHttpClient() => HttpClient()
    ..connectionTimeout = ApiTimeouts.connect
    ..idleTimeout = const Duration(seconds: 20)
    ..maxConnectionsPerHost = 6
    ..userAgent = 'ComandOS-Android';

  Uri _uri(String path, [Map<String, String>? query]) =>
      server.baseUri.replace(path: path, queryParameters: query);

  /// Sends one request and decodes the JSON answer. Completing [cancel]
  /// aborts it like a timeout (used to end a long-poll right away).
  Future<ApiResponse> request(
    String method,
    String path, {
    Map<String, String>? query,
    Map<String, Object?>? body,
    Duration timeout = ApiTimeouts.get,
    Future<void>? cancel,
  }) async {
    if (_closed) throw const NetworkException();
    final Completer<void> abort = Completer<void>();
    final Timer deadline = Timer(timeout, () {
      if (!abort.isCompleted) abort.complete();
    });
    cancel?.then((_) {
      if (!abort.isCompleted) abort.complete();
    });
    try {
      final http.AbortableRequest req =
          http.AbortableRequest(method, _uri(path, query), abortTrigger: abort.future)
            ..headers['X-Comandos-Token'] = server.token
            ..headers['Accept'] = 'application/json';
      if (body != null) {
        req.headers['Content-Type'] = 'application/json';
        req.bodyBytes = utf8.encode(jsonEncode(body));
      }
      final http.StreamedResponse streamed = await _client.send(req);
      final Uint8List bytes = await streamed.stream.toBytes();
      Object? json;
      bool malformed = false;
      try {
        json = await decodeJsonBody(bytes);
      } on FormatException {
        malformed = true;
      }
      return ApiResponse(streamed.statusCode, json, malformed: malformed);
    } on http.RequestAbortedException {
      throw const RequestTimeoutException();
    } on SocketException catch (e) {
      throw NetworkException(dns: e.message.contains('host lookup') ||
          (e.osError?.message.contains('No address') ?? false));
    } on HandshakeException {
      throw const NetworkException();
    } on TlsException {
      throw const NetworkException();
    } on HttpException {
      throw const NetworkException();
    } on http.ClientException catch (e) {
      if (abort.isCompleted) throw const RequestTimeoutException();
      throw NetworkException(dns: e.message.contains('host lookup'));
    } finally {
      deadline.cancel();
    }
  }

  /// `POST /tts`: Edge's neural voice rendered on the desktop (mp3 bytes).
  /// Anything but audio (no edge-tts there, the service failed) throws.
  Future<Uint8List> speech({
    required String text,
    required String voice,
    required String rate,
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (_closed) throw const NetworkException();
    final Completer<void> abort = Completer<void>();
    final Timer deadline = Timer(timeout, () {
      if (!abort.isCompleted) abort.complete();
    });
    try {
      final http.AbortableRequest req = http.AbortableRequest('POST', _uri('/tts'), abortTrigger: abort.future)
        ..headers['X-Comandos-Token'] = server.token
        ..headers['Accept'] = 'audio/mpeg'
        ..headers['Content-Type'] = 'application/json'
        ..bodyBytes = utf8.encode(jsonEncode(<String, Object?>{'text': text, 'voice': voice, 'rate': rate}));
      final http.StreamedResponse streamed = await _client.send(req);
      final Uint8List bytes = await streamed.stream.toBytes();
      final String type = streamed.headers['content-type'] ?? '';
      if (streamed.statusCode != 200 || !type.startsWith('audio/') || bytes.isEmpty) {
        throw const BadResponseException();
      }
      return bytes;
    } on http.RequestAbortedException {
      throw const RequestTimeoutException();
    } on SocketException {
      throw const NetworkException();
    } on http.ClientException {
      if (abort.isCompleted) throw const RequestTimeoutException();
      throw const NetworkException();
    } finally {
      deadline.cancel();
    }
  }

  /// Like [request] but throws a typed [ApiException] for any non-2xx.
  Future<ApiResponse> requestOk(
    String method,
    String path, {
    Map<String, String>? query,
    Map<String, Object?>? body,
    Duration timeout = ApiTimeouts.get,
    Future<void>? cancel,
  }) async {
    final ApiResponse r =
        await request(method, path, query: query, body: body, timeout: timeout, cancel: cancel);
    if (!r.ok) throw exceptionFor(r);
    return r;
  }

  // ---- Typed routes -------------------------------------------------------

  /// `GET /session-feed`.
  Future<SessionFeed> sessionFeed() async {
    final ApiResponse r = await requestOk('GET', '/session-feed');
    final SessionFeed? feed = SessionFeed.tryParse(r.json);
    if (feed == null) throw const BadResponseException();
    return feed;
  }

  /// `POST /chat/transcript`. [since] is the previous `token` ('' at first).
  Future<TranscriptResponse> transcript({
    required String session,
    String? pane,
    String since = '',
  }) async {
    final ApiResponse r = await requestOk('POST', '/chat/transcript', body: <String, Object?>{
      'session': session,
      'pane': ?pane,
      'since': since,
    });
    final TranscriptResponse? t = TranscriptResponse.tryParse(r.json);
    if (t == null) throw const BadResponseException();
    return t;
  }

  /// `POST /session-seen`.
  Future<void> sessionSeen(String session) =>
      requestOk('POST', '/session-seen', body: <String, Object?>{'session': session});

  /// `POST /key`. Allowed keys: Enter Escape Up Down Tab y n 1-9.
  Future<void> key({required String session, String? pane, required String key}) =>
      requestOk('POST', '/key', body: <String, Object?>{
        'session': session,
        'pane': ?pane,
        'key': key,
      });

  /// `POST /chat/answer`: the desktop reads the pane again and, only if the
  /// menu is still [sig], moves the cursor to [row] with ↑/↓ (typing [text] for
  /// «Type something») and presses Enter. Null when answered, else the reason
  /// in Spanish. Throws [NotFoundException] when the desktop predates the route.
  Future<String?> answerRow({
    required String session,
    String? pane,
    required String sig,
    required int row,
    String? text,
  }) async {
    final ApiResponse r = await request('POST', '/chat/answer', body: <String, Object?>{
      'session': session,
      'pane': ?pane,
      'sig': sig,
      'row': row,
      if (text != null && text.trim().isNotEmpty) 'text': text.trim(),
    });
    if (r.ok) return null;
    final Object? why = r.map['error'];
    if (r.status == 401) throw const UnauthorizedException();
    if (r.status == 404 && !(why is String && why.contains('pane'))) throw NotFoundException(code: r.code);
    return why is String && why.isNotEmpty ? why : 'No se pudo responder.';
  }

  /// Answers a permission menu like the web client: the digit, then Enter
  /// 250 ms later.
  Future<void> answerAsk({required String session, String? pane, required String option}) async {
    await key(session: session, pane: pane, key: option);
    await Future<void>.delayed(const Duration(milliseconds: 250));
    await key(session: session, pane: pane, key: 'Enter');
  }

  /// Releases the connection pool. The instance is unusable afterwards.
  void close() {
    _closed = true;
    _client.close();
  }
}
