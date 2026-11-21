import 'paired_server.dart';

/// Why a pairing input was rejected. The UI maps each to Spanish copy.
enum PairingParseError {
  empty,
  notAUrl,
  notHttps,
  missingToken,
  badToken,
  hostIsIp,
  hostNotAllowed,
}

/// Spanish explanation for a [PairingParseError].
String pairingParseErrorMessage(PairingParseError error) {
  switch (error) {
    case PairingParseError.empty:
      return 'Pega el enlace del panel «Remoto» o escanea su código QR.';
    case PairingParseError.notAUrl:
      return 'Eso no parece un enlace. Debe verse así: '
          'https://mi-equipo.mi-red.ts.net/?token=…';
    case PairingParseError.notHttps:
      return 'El enlace debe empezar con https://. Cópialo tal cual del panel «Remoto».';
    case PairingParseError.missingToken:
      return 'Al enlace le falta el token (?token=…). Cópialo completo del panel «Remoto».';
    case PairingParseError.badToken:
      return 'El token del enlace está incompleto o tiene caracteres extraños. Vuelve a copiarlo.';
    case PairingParseError.hostIsIp:
      return 'Usa el nombre MagicDNS de tu equipo (termina en .ts.net), no su IP. '
          'ComandOS rechaza las IP por seguridad.';
    case PairingParseError.hostNotAllowed:
      return 'Esa dirección no es de Tailscale. ComandOS solo acepta nombres '
          'MagicDNS como mi-equipo.mi-red.ts.net.';
  }
}

/// Result of [parsePairingInput]: either a server or an error.
class PairingParseResult {
  const PairingParseResult.ok(PairedServer this.server) : error = null;
  const PairingParseResult.fail(PairingParseError this.error) : server = null;

  final PairedServer? server;
  final PairingParseError? error;

  bool get isOk => server != null;
}

final RegExp _tokenChars = RegExp(r'^[A-Za-z0-9_\-.~=+/]+$');
final RegExp _ipv4 = RegExp(r'^\d{1,3}(\.\d{1,3}){3}$');

/// Parses what the desktop "Remoto" panel shows (QR or copied link):
/// `https://<host>/?token=<TOKEN>`. Also tolerates surrounding whitespace,
/// a missing scheme, and the terminal link variant (`?auth=<TOKEN>`).
PairingParseResult parsePairingInput(String raw) {
  String input = raw.trim();
  if (input.isEmpty) return const PairingParseResult.fail(PairingParseError.empty);
  // A QR or a chat app may wrap the link in <…> or quotes.
  input = input.replaceAll(RegExp(r'^[<"\x27]+|[>"\x27]+$'), '');
  if (!input.contains('://')) {
    if (!input.contains('.') || input.contains(' ')) {
      return const PairingParseResult.fail(PairingParseError.notAUrl);
    }
    input = 'https://$input';
  }
  final Uri? uri = Uri.tryParse(input);
  if (uri == null || uri.host.isEmpty) {
    return const PairingParseResult.fail(PairingParseError.notAUrl);
  }
  if (uri.scheme.toLowerCase() != 'https') {
    return const PairingParseResult.fail(PairingParseError.notHttps);
  }
  final String host = uri.host.toLowerCase();
  if (_ipv4.hasMatch(host) || host.contains(':') || input.contains('[')) {
    return const PairingParseResult.fail(PairingParseError.hostIsIp);
  }
  final List<String> labels = host.split('.');
  if (!host.endsWith('.ts.net') || labels.length < 3 || labels.any((String l) => l.isEmpty)) {
    return const PairingParseResult.fail(PairingParseError.hostNotAllowed);
  }
  final String? token = uri.queryParameters['token'] ?? uri.queryParameters['auth'];
  if (token == null || token.isEmpty) {
    return const PairingParseResult.fail(PairingParseError.missingToken);
  }
  if (token.length < 16 || token.length > 256 || !_tokenChars.hasMatch(token)) {
    return const PairingParseResult.fail(PairingParseError.badToken);
  }
  final int? port = uri.hasPort && uri.port != 443 ? uri.port : null;
  return PairingParseResult.ok(PairedServer(host: host, token: token, port: port));
}
