/// The desktop this phone is paired with: its MagicDNS host and access token.
///
/// [toString] never prints the token. Do not log [token] anywhere.
class PairedServer {
  const PairedServer({required this.host, required this.token, this.port});

  /// MagicDNS name, e.g. `mi-equipo.mi-red.ts.net`.
  final String host;

  /// Explicit HTTPS port when not 443.
  final int? port;

  /// Shared dashboard token. Secret.
  final String token;

  Uri get baseUri => Uri(scheme: 'https', host: host, port: port);

  /// Host plus port, as shown to the person.
  String get displayHost => port == null ? host : '$host:$port';

  Map<String, Object?> toJson() => <String, Object?>{
        'host': host,
        'port': port,
        'token': token,
      };

  static PairedServer? fromJson(Object? json) {
    if (json is! Map) return null;
    final Object? host = json['host'];
    final Object? token = json['token'];
    final Object? port = json['port'];
    if (host is! String || host.isEmpty || token is! String || token.isEmpty) {
      return null;
    }
    return PairedServer(host: host, token: token, port: port is int ? port : null);
  }

  @override
  String toString() => 'PairedServer($displayHost, token: ***)';
}
