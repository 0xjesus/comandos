import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/config/pairing_parser.dart';
import 'package:flutter_test/flutter_test.dart';

const String token = 'TOKEN_abcdefghijklmnopqrstuvwxyz-0123456789';

void main() {
  PairedServer ok(String input) {
    final PairingParseResult r = parsePairingInput(input);
    expect(r.isOk, isTrue, reason: 'should parse: $input (${r.error})');
    return r.server!;
  }

  PairingParseError fail(String input) {
    final PairingParseResult r = parsePairingInput(input);
    expect(r.isOk, isFalse, reason: 'should fail: $input');
    return r.error!;
  }

  test('the Remoto URL', () {
    final PairedServer s = ok('https://mi-equipo.mi-red.ts.net/?token=$token');
    expect(s.host, 'mi-equipo.mi-red.ts.net');
    expect(s.token, token);
    expect(s.port, isNull);
    expect(s.baseUri.toString(), 'https://mi-equipo.mi-red.ts.net');
  });

  test('tolerates whitespace, quotes, case and a missing scheme', () {
    expect(ok('  "https://Mi-Equipo.mi-red.ts.net/?token=$token"\n').host,
        'mi-equipo.mi-red.ts.net');
    expect(ok('mi-equipo.mi-red.ts.net/?token=$token').host, 'mi-equipo.mi-red.ts.net');
  });

  test('accepts the terminal link (auth=) and a custom port', () {
    final PairedServer s = ok('https://mi-equipo.mi-red.ts.net:8443/term/?auth=$token');
    expect(s.token, token);
    expect(s.port, 8443);
    expect(s.displayHost, 'mi-equipo.mi-red.ts.net:8443');
  });

  test('the token never appears in toString', () {
    expect(ok('https://mi-equipo.mi-red.ts.net/?token=$token').toString(), isNot(contains(token)));
  });

  test('rejections with a reason', () {
    expect(fail(''), PairingParseError.empty);
    expect(fail('hola qué tal'), PairingParseError.notAUrl);
    expect(fail('http://mi-equipo.mi-red.ts.net/?token=$token'), PairingParseError.notHttps);
    expect(fail('https://mi-equipo.mi-red.ts.net/'), PairingParseError.missingToken);
    expect(fail('https://mi-equipo.mi-red.ts.net/?token=corto'), PairingParseError.badToken);
    expect(fail('https://mi-equipo.mi-red.ts.net/?token=con%20espacios%20y%20mas%20texto'),
        PairingParseError.badToken);
    expect(fail('https://100.64.0.1/?token=$token'), PairingParseError.hostIsIp);
    expect(fail('https://[fd7a:115c:a1e0::1]/?token=$token'), PairingParseError.hostIsIp);
    expect(fail('https://ejemplo.com/?token=$token'), PairingParseError.hostNotAllowed);
    expect(fail('https://ts.net/?token=$token'), PairingParseError.hostNotAllowed);
  });

  test('every error has Spanish copy', () {
    for (final PairingParseError e in PairingParseError.values) {
      expect(pairingParseErrorMessage(e), isNotEmpty);
    }
  });

  test('stored JSON round-trips', () {
    final PairedServer s = ok('https://mi-equipo.mi-red.ts.net:8443/?token=$token');
    final PairedServer? back = PairedServer.fromJson(s.toJson());
    expect(back!.host, s.host);
    expect(back.port, 8443);
    expect(back.token, token);
    expect(PairedServer.fromJson(<String, Object?>{'host': 'x'}), isNull);
  });
}
