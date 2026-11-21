import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/config/paired_server.dart';

/// Checks a candidate pairing with `GET /session-feed`. Returns null when the
/// desktop accepted the token, otherwise Spanish copy saying what to do.
Future<String?> checkPairing(
  PairedServer server, {
  ComandosApi Function(PairedServer server)? apiFactory,
}) async {
  final ComandosApi api = (apiFactory ?? ComandosApi.new)(server);
  try {
    final ApiResponse r = await api.request('GET', '/session-feed');
    if (r.ok) {
      if (r.malformed || r.map['sessions'] is! List) {
        return 'Respondió ${server.displayHost}, pero no parece ComandOS. '
            'Revisa que el enlace sea el del panel «Remoto».';
      }
      return null;
    }
    switch (r.status) {
      case 401:
        return 'Tu equipo rechazó el token. Puede que haya cambiado: '
            'abre «Remoto» en el equipo y escanea el código nuevo.';
      case 403:
        return 'Tu equipo no acepta esta dirección. Usa el nombre MagicDNS '
            '(termina en .ts.net), no una IP.';
      case 503:
        // Auth passed (the gate runs before the handler): pairing is valid.
        return null;
      case 502:
        return 'Tailscale llega a tu equipo, pero ComandOS no está corriendo ahí. '
            'Ábrelo en el equipo y vuelve a intentar.';
    }
    return 'Tu equipo respondió ${r.status}. Revisa que ComandOS esté actualizado.';
  } on RequestTimeoutException {
    return 'Tu equipo no respondió a tiempo. ¿Tailscale está encendido en este '
        'teléfono y en el equipo?';
  } on NetworkException catch (e) {
    return '${e.userMessage} (Abre la app de Tailscale y comprueba que diga «Connected».)';
  } on ApiException catch (e) {
    return e.userMessage;
  } finally {
    api.close();
  }
}
