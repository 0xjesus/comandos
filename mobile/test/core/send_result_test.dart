import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/outbox/send_result.dart';
import 'package:flutter_test/flutter_test.dart';

SendResult c(int status, [Object? json]) => classifyMessageResponse(ApiResponse(status, json));
SendResult l(int status) => classifyLegacyResponse(ApiResponse(status, null));

void main() {
  test('POST /session/message answers', () {
    expect(c(200, <String, Object?>{'ok': true, 'status': 'delivered', 'duplicate': true}),
        isA<Delivered>().having((Delivered d) => d.duplicate, 'duplicate', isTrue));
    expect(c(202, <String, Object?>{'ok': true, 'status': 'delivering'}), isA<InProgress>());
    expect(c(404, <String, Object?>{'ok': false, 'code': 'pane_gone', 'status': 'rejected'}),
        isA<Rejected>().having((Rejected r) => r.code, 'code', 'pane_gone'));
    expect(c(404, <String, Object?>{'ok': false, 'code': 'session_gone'}),
        isA<Rejected>().having((Rejected r) => r.code, 'code', 'session_gone'));
    expect(c(404, <String, Object?>{'status': 'unknown'}), isA<UnknownReceipt>());
    expect(c(409, <String, Object?>{'code': 'pane_busy'}), isA<Busy>());
    expect(c(429), isA<Busy>());
    expect(c(422, <String, Object?>{'code': 'invalid'}), isA<Rejected>());
    expect(c(400), isA<Rejected>());
    expect(c(401), isA<Rejected>().having((Rejected r) => r.code, 'code', 'unauthorized'));
    expect(c(403), isA<Rejected>().having((Rejected r) => r.code, 'code', 'forbidden'));
    expect(c(500, <String, Object?>{'ok': false, 'status': 'uncertain', 'code': 'partial'}), isA<Uncertain>());
    expect(c(502), isA<TransientFailure>());
    expect(c(504, <String, Object?>{'error': 'Tiempo de espera agotado'}), isA<TransientFailure>());
    expect(c(200, 'not a map'), isA<TransientFailure>());
    // GET receipt shapes: 200 with any stored state.
    expect(c(200, <String, Object?>{'ok': true, 'status': 'delivering', 'clientMessageId': 'x'}),
        isA<InProgress>());
    expect(c(200, <String, Object?>{'ok': false, 'status': 'uncertain', 'code': 'enter_failed'}),
        isA<Uncertain>());
    expect(c(200, <String, Object?>{'ok': true, 'status': 'delivered', 'pane': '%3'}),
        isA<Delivered>().having((Delivered d) => d.pane, 'pane', '%3'));
    // Rejections that leave no receipt: retry the same id later.
    expect(c(500, <String, Object?>{'ok': false, 'status': 'rejected', 'code': 'tmux_failed'}),
        isA<TransientFailure>());
    expect(c(503, <String, Object?>{'ok': false, 'status': 'rejected', 'code': 'unavailable'}),
        isA<TransientFailure>());
  });

  test('a 404 without a code means the route does not exist (old server)', () {
    expect(c(404, <String, Object?>{'error': 'Not found'}), isA<RouteMissing>());
    expect(c(404), isA<RouteMissing>());
    expect(c(405), isA<RouteMissing>());
  });

  test('legacy /send answers', () {
    expect(l(200), isA<Delivered>());
    expect(l(404), isA<Rejected>());
    expect(l(502), isA<Rejected>());
    expect(l(500), isA<Uncertain>());
    expect(l(504), isA<Uncertain>());
  });
}
