import 'dart:async';
import 'dart:io';

import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_item.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/outbox/send_result.dart';
import 'package:flutter_test/flutter_test.dart';

const String host = 'mi-equipo.mi-red.ts.net';

/// Scripted transport: each call pops the next result for its method
/// (default: delivered / unknown / null) and records what was called.
class FakeTransport implements MessageTransport {
  final List<SendResult> sendResults = <SendResult>[];
  final List<SendResult> lookupResults = <SendResult>[];
  final List<SendResult> legacyResults = <SendResult>[];
  final List<int?> counts = <int?>[];
  final List<String> calls = <String>[];

  /// When set, `send` waits on it (to test what happens mid-flight).
  Completer<void>? gate;

  @override
  Future<SendResult> send(OutboxItem item) async {
    calls.add('send:${item.id}:${item.session}:${item.text}');
    if (gate != null) await gate!.future;
    return sendResults.isEmpty ? const Delivered() : sendResults.removeAt(0);
  }

  @override
  Future<SendResult> lookup(OutboxItem item) async {
    calls.add('lookup:${item.id}');
    return lookupResults.isEmpty ? const UnknownReceipt() : lookupResults.removeAt(0);
  }

  @override
  Future<SendResult> legacySend(OutboxItem item) async {
    calls.add('legacy:${item.id}');
    return legacyResults.isEmpty ? const Delivered() : legacyResults.removeAt(0);
  }

  @override
  Future<int?> countUserMessages(String session, String? pane, String text) async {
    calls.add('count:$session');
    return counts.isEmpty ? null : counts.removeAt(0);
  }

  List<String> get sends => calls.where((String c) => c.startsWith('send:')).toList();
}

class _NoTimer implements Timer {
  @override
  void cancel() {}
  @override
  bool get isActive => false;
  @override
  int get tick => 0;
}

class Harness {
  Harness({OutboxStore? store}) : store = store ?? MemoryOutboxStore() {
    outbox = build();
  }

  final OutboxStore store;
  final FakeTransport transport = FakeTransport();
  int nowMs = 1000000;
  int _ids = 0;
  late Outbox outbox;

  Outbox build() => Outbox(
        store: store,
        transport: transport,
        now: () => nowMs,
        timer: (Duration d, void Function() cb) => _NoTimer(),
        newId: () => 'id-${++_ids}',
      );

  Future<void> start() async {
    await outbox.load();
    outbox.setHost(host);
  }

  void advance(Duration d) => nowMs += d.inMilliseconds;

  OutboxItem only() => outbox.items.single;
}

void main() {
  group('Outbox', () {
    test('send is durable before delivery starts, then delivered', () async {
      final Harness h = Harness();
      await h.start();
      final MemoryOutboxStore store = h.store as MemoryOutboxStore;
      final OutboxItem item = await h.outbox.enqueue(session: 's1', pane: '%1', text: 'hola');
      expect(store.saved.map((OutboxItem i) => i.id), contains(item.id));
      await h.outbox.pump();
      expect(h.transport.sends, <String>['send:id-1:s1:hola']);
      expect(h.only().state, OutboxState.delivered);
    });

    test('a failed disk write rejects the send and keeps nothing', () async {
      final Harness h = Harness();
      await h.start();
      (h.store as MemoryOutboxStore).failNext = true;
      await expectLater(
        h.outbox.enqueue(session: 's1', text: 'hola'),
        throwsA(isA<OutboxRejection>()),
      );
      await h.outbox.pump();
      expect(h.outbox.items, isEmpty);
      expect(h.transport.sends, isEmpty);
    });

    test('duplicate replay from the server counts as delivered', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const Delivered(duplicate: true));
      await h.outbox.enqueue(session: 's1', text: 'hola');
      await h.outbox.pump();
      expect(h.only().state, OutboxState.delivered);
      expect(h.transport.sends, hasLength(1));
    });

    test('retry after a network error keeps the same clientMessageId', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const TransientFailure('sin red'));
      await h.outbox.enqueue(session: 's1', text: 'hola');
      await h.outbox.pump();
      expect(h.only().state, OutboxState.queued);
      expect(h.only().ambiguous, isTrue);

      // Not due yet: nothing happens.
      await h.outbox.pump();
      expect(h.transport.sends, hasLength(1));

      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      // Ambiguous failure: ask for the receipt first, then re-POST the same id.
      expect(h.transport.calls, <String>[
        'send:id-1:s1:hola',
        'lookup:id-1',
        'send:id-1:s1:hola',
      ]);
      expect(h.only().state, OutboxState.delivered);
      expect(h.only().id, 'id-1');
    });

    test('a receipt from the lookup settles it without sending again', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const TransientFailure('timeout'));
      h.transport.lookupResults.add(const Delivered());
      await h.outbox.enqueue(session: 's1', text: 'hola');
      await h.outbox.pump();
      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      expect(h.transport.sends, hasLength(1));
      expect(h.only().state, OutboxState.delivered);
    });

    test('backoff follows 1, 2, 5, 10, 30, then 60 s', () async {
      final Harness h = Harness();
      await h.start();
      for (int i = 0; i < 8; i++) {
        h.transport.sendResults.add(const TransientFailure('x'));
      }
      await h.outbox.enqueue(session: 's1', text: 'hola');
      final List<int> gaps = <int>[];
      await h.outbox.pump();
      for (int i = 0; i < 6; i++) {
        gaps.add(h.only().nextAttemptAtMs - h.nowMs);
        h.nowMs = h.only().nextAttemptAtMs;
        await h.outbox.pump();
      }
      expect(gaps, <int>[1000, 2000, 5000, 10000, 30000, 60000]);
    });

    test('nudge (connectivity back / resume) retries immediately', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.addAll(<SendResult>[
        const TransientFailure('x'),
        const TransientFailure('x'),
        const TransientFailure('x'),
      ]);
      await h.outbox.enqueue(session: 's1', text: 'hola');
      await h.outbox.pump();
      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      h.advance(const Duration(seconds: 2));
      await h.outbox.pump(); // now waiting 10 s
      expect(h.only().nextAttemptAtMs, greaterThan(h.nowMs));
      h.outbox.nudge();
      await h.outbox.pump();
      expect(h.only().state, OutboxState.delivered);
    });

    test('strict FIFO per session; other sessions are independent', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const TransientFailure('x')); // A1 fails once
      await h.outbox.enqueue(session: 'A', text: 'a1');
      await h.outbox.enqueue(session: 'A', text: 'a2');
      await h.outbox.enqueue(session: 'B', text: 'b1');
      await h.outbox.pump();
      expect(h.transport.sends, <String>['send:id-1:A:a1', 'send:id-3:B:b1']);
      expect(h.outbox.byId('id-2')!.state, OutboxState.queued);

      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      expect(h.transport.sends, <String>[
        'send:id-1:A:a1',
        'send:id-3:B:b1',
        'send:id-1:A:a1',
        'send:id-2:A:a2',
      ]);
      expect(h.outbox.items.every((OutboxItem i) => i.state == OutboxState.delivered), isTrue);
    });

    test('pane_gone is not retried and blocks the session until discarded', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const Rejected(FailCode.paneGone));
      await h.outbox.enqueue(session: 'A', pane: '%9', text: 'a1');
      await h.outbox.enqueue(session: 'A', text: 'a2');
      await h.outbox.pump();
      final OutboxItem first = h.outbox.byId('id-1')!;
      expect(first.state, OutboxState.failed);
      expect(first.failCode, FailCode.paneGone);
      expect(outboxFailureMessage(first.failCode), contains('panel'));

      h.advance(const Duration(minutes: 10));
      await h.outbox.pump();
      expect(h.transport.sends, hasLength(1), reason: 'never auto-retried, a2 waits');

      await h.outbox.discard('id-1');
      await h.outbox.pump();
      expect(h.transport.sends.last, 'send:id-2:A:a2');
    });

    test('manual retry after a rejection uses a fresh id and the new pane', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const Rejected(FailCode.paneGone));
      await h.outbox.enqueue(session: 'A', pane: '%9', text: 'a1');
      await h.outbox.pump();
      await h.outbox.retry('id-1', pane: '%3');
      await h.outbox.pump();
      expect(h.only().id, 'id-2');
      expect(h.only().pane, '%3');
      expect(h.only().state, OutboxState.delivered);
    });

    test('401 and 422 fail without retry', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.addAll(<SendResult>[
        const Rejected(FailCode.unauthorized),
        const Rejected(FailCode.invalid),
      ]);
      await h.outbox.enqueue(session: 'A', text: 'a');
      await h.outbox.enqueue(session: 'B', text: 'b');
      await h.outbox.pump();
      h.advance(const Duration(minutes: 5));
      await h.outbox.pump();
      expect(h.transport.sends, hasLength(2));
      expect(h.outbox.attentionCount, 2);
    });

    test('409 pane_busy retries with a short backoff', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.addAll(<SendResult>[const Busy(), const Busy()]);
      await h.outbox.enqueue(session: 'A', text: 'a');
      await h.outbox.pump();
      expect(h.only().nextAttemptAtMs - h.nowMs, lessThanOrEqualTo(1000));
      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      expect(h.only().state, OutboxState.delivered);
      expect(h.transport.sends.toSet(), <String>{'send:id-1:A:a'});
    });

    test('202 delivering asks for the receipt next time', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const InProgress());
      h.transport.lookupResults.add(const Delivered());
      await h.outbox.enqueue(session: 'A', text: 'a');
      await h.outbox.pump();
      h.advance(const Duration(seconds: 1));
      await h.outbox.pump();
      expect(h.transport.calls, <String>['send:id-1:A:a', 'lookup:id-1']);
      expect(h.only().state, OutboxState.delivered);
    });

    test('old server: legacy /send once, network error → uncertain, never resent', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const RouteMissing());
      h.transport.legacyResults.add(const Uncertain());
      await h.outbox.enqueue(session: 'A', text: 'a1', transcriptBaseline: 0);
      await h.outbox.enqueue(session: 'A', text: 'a2', transcriptBaseline: 0);
      await h.outbox.pump();
      expect(h.outbox.byId('id-1')!.state, OutboxState.uncertain);
      expect(h.outbox.byId('id-1')!.legacyTried, isTrue);

      for (int i = 0; i < 12; i++) {
        h.advance(const Duration(seconds: 20));
        await h.outbox.pump();
      }
      expect(h.transport.calls.where((String c) => c.startsWith('legacy:')), hasLength(1));
      expect(h.outbox.byId('id-1')!.state, OutboxState.uncertain);
      expect(h.outbox.byId('id-2')!.state, OutboxState.queued, reason: 'FIFO holds a2');
    });

    test('uncertain resolves to delivered when the transcript shows it', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.add(const RouteMissing());
      h.transport.legacyResults.add(const Uncertain());
      h.transport.counts.add(1); // first check: the message appeared
      await h.outbox.enqueue(session: 'A', text: 'a1', transcriptBaseline: 0);
      await h.outbox.pump();
      h.advance(const Duration(seconds: 5));
      await h.outbox.pump();
      expect(h.only().state, OutboxState.delivered);
    });

    test('legacy 200 → delivered; long text refused instead of truncated', () async {
      final Harness h = Harness();
      await h.start();
      h.transport.sendResults.addAll(<SendResult>[const RouteMissing(), const RouteMissing()]);
      await h.outbox.enqueue(session: 'A', text: 'corto', transcriptBaseline: 0);
      await h.outbox.enqueue(session: 'B', text: 'x' * 5000, transcriptBaseline: 0);
      await h.outbox.pump();
      expect(h.outbox.byId('id-1')!.state, OutboxState.delivered);
      expect(h.outbox.byId('id-2')!.state, OutboxState.failed);
      expect(h.outbox.byId('id-2')!.failCode, FailCode.legacyTooLong);
      expect(h.transport.calls.where((String c) => c.startsWith('legacy:')), hasLength(1));
    });

    test('text over 20000 chars is refused, never truncated', () async {
      final Harness h = Harness();
      await h.start();
      await expectLater(
        h.outbox.enqueue(session: 'A', text: 'x' * 20001),
        throwsA(isA<OutboxRejection>()),
      );
      expect(h.outbox.items, isEmpty);
    });

    test('delivered bubble hides once the transcript shows the message', () async {
      final Harness h = Harness();
      await h.start();
      await h.outbox.enqueue(session: 'A', text: 'sí', transcriptBaseline: 1);
      await h.outbox.enqueue(session: 'A', text: 'sí', transcriptBaseline: 1);
      await h.outbox.pump();
      // An older identical "sí" alone does not count.
      h.outbox.observeTranscript('A', <String>['sí']);
      expect(h.outbox.items, hasLength(2));
      // One new "sí" settles only the first of the two.
      h.outbox.observeTranscript('A', <String>['sí', 'sí']);
      expect(h.outbox.items.single.id, 'id-2');
      h.outbox.observeTranscript('A', <String>['sí', 'sí', 'sí']);
      expect(h.outbox.items, isEmpty);
    });

    test('delivered bubble expires after its linger time', () async {
      final Harness h = Harness();
      await h.start();
      await h.outbox.enqueue(session: 'A', text: 'hola');
      await h.outbox.pump();
      h.advance(Outbox.deliveredLinger);
      await h.outbox.pump();
      expect(h.outbox.items, isEmpty);
    });

    test('never sent to a different host', () async {
      final Harness h = Harness();
      await h.start();
      h.outbox.setHost(null); // unpaired
      await expectLater(
        h.outbox.enqueue(session: 'A', text: 'x'),
        throwsA(isA<OutboxRejection>()),
      );
      h.outbox.setHost(host);
      h.transport.gate = Completer<void>();
      h.transport.sendResults.add(const TransientFailure('x'));
      await h.outbox.enqueue(session: 'A', text: 'x');
      h.transport.gate!.complete();
      await h.outbox.pump();
      h.outbox.setHost('otro.mi-red.ts.net');
      h.advance(const Duration(seconds: 5));
      await h.outbox.pump();
      expect(h.only().state, OutboxState.failed);
      expect(h.only().failCode, FailCode.otherHost);
      expect(h.transport.sends, hasLength(1));
    });
  });

  group('Persistence', () {
    late Directory dir;

    setUp(() async => dir = await Directory.systemTemp.createTemp('outbox_test'));
    tearDown(() async => dir.delete(recursive: true));

    test('queue survives a restart; an in-flight item resumes with the same id', () async {
      final String path = '${dir.path}/outbox.json';
      final Harness first = Harness(store: FileOutboxStore(path));
      await first.start();
      first.transport.gate = Completer<void>(); // "process dies" mid-send
      await first.outbox.enqueue(session: 'A', pane: '%1', text: 'línea 1\nlínea 2');
      await first.outbox.enqueue(session: 'A', text: 'segundo');
      first.outbox.pump(); // ignore: unawaited_futures
      await Future<void>.delayed(Duration.zero);
      expect(first.only, throwsStateError); // two items
      expect(first.outbox.byId('id-1')!.state, OutboxState.sending);

      // A new process reads the same file.
      final Harness second = Harness(store: FileOutboxStore(path));
      await second.outbox.load();
      final List<OutboxItem> items = second.outbox.items;
      expect(items.map((OutboxItem i) => i.text), <String>['línea 1\nlínea 2', 'segundo']);
      expect(items.first.id, 'id-1');
      expect(items.first.state, OutboxState.queued);
      expect(items.first.ambiguous, isTrue);
      second.outbox.setHost(host);
      await second.outbox.pump();
      expect(second.transport.calls.first, 'lookup:id-1');
      expect(second.transport.sends.first, 'send:id-1:A:línea 1\nlínea 2');
      expect(second.outbox.items.every((OutboxItem i) => i.state == OutboxState.delivered), isTrue);
      // The first "process" is dead: its gated send never completes.
    });

    test('a legacy send in flight at death comes back uncertain', () async {
      final String path = '${dir.path}/outbox.json';
      final FileOutboxStore store = FileOutboxStore(path);
      await store.save(<OutboxItem>[
        const OutboxItem(
          id: 'x',
          host: host,
          session: 'A',
          text: 'hola',
          createdAtMs: 1,
          state: OutboxState.sending,
          legacyTried: true,
        ),
      ]);
      final Harness h = Harness(store: FileOutboxStore(path));
      await h.start();
      expect(h.only().state, OutboxState.uncertain);
      await h.outbox.pump();
      expect(h.transport.calls.where((String c) => !c.startsWith('count')), isEmpty);
    });

    test('a corrupt file is moved aside, not deleted', () async {
      final String path = '${dir.path}/outbox.json';
      await File(path).writeAsString('{not json');
      final List<OutboxItem> items = await FileOutboxStore(path).load();
      expect(items, isEmpty);
      final List<String> names =
          dir.listSync().map((FileSystemEntity e) => e.uri.pathSegments.last).toList();
      expect(names.any((String n) => n.startsWith('outbox.json.corrupt-')), isTrue);
    });

    test('drafts are restored after a restart', () async {
      final String path = '${dir.path}/drafts.json';
      final DraftStore a = DraftStore(path, debounce: const Duration(milliseconds: 1));
      await a.load();
      a.set('A', 'borrador a medias');
      a.set('B', 'otro');
      a.set('B', '');
      await a.flush();
      final DraftStore b = DraftStore(path);
      await b.load();
      expect(b.get('A'), 'borrador a medias');
      expect(b.get('B'), '');
      a.dispose();
      b.dispose();
    });
  });

  group('transcript matching', () {
    test('normalises whitespace and handles the server clip', () {
      expect(transcriptTextMatches('  hola\n mundo ', 'hola mundo'), isTrue);
      final String long = 'a' * 7000;
      expect(transcriptTextMatches('${'a' * 6000}…', long), isTrue);
      expect(transcriptTextMatches('hola', 'hola!'), isFalse);
    });
  });
}
