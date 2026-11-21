import 'package:flutter/material.dart';

import '../../core/config/paired_server.dart';
import '../../core/config/pairing_parser.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../ui/theme.dart';
import '../notifications/notifications_hook.dart';
import 'pairing_check.dart';
import 'qr_scan_screen.dart';

/// First screen when unpaired; also reached from Ajustes → "Volver a vincular".
class PairingScreen extends StatefulWidget {
  const PairingScreen({super.key, this.onPaired});

  static const String routeName = '/pairing';

  /// Called after a successful pairing (the app navigates to the sessions).
  final VoidCallback? onPaired;

  @override
  State<PairingScreen> createState() => _PairingScreenState();
}

class _PairingScreenState extends State<PairingScreen> {
  final TextEditingController _link = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _link.dispose();
    super.dispose();
  }

  Future<void> _scan() async {
    final String? value = await Navigator.of(context).push<String>(
      MaterialPageRoute<String>(builder: (_) => const QrScanScreen()),
    );
    if (!mounted || value == null) return;
    _link.text = value;
    await _submit();
  }

  Future<void> _submit() async {
    if (_busy) return;
    final PairingParseResult parsed = parsePairingInput(_link.text);
    if (!parsed.isOk) {
      setState(() => _error = pairingParseErrorMessage(parsed.error!));
      return;
    }
    final PairedServer server = parsed.server!;
    setState(() {
      _busy = true;
      _error = null;
    });
    final String? problem = await checkPairing(server);
    if (!mounted) return;
    if (problem != null) {
      setState(() {
        _busy = false;
        _error = problem;
      });
      return;
    }
    try {
      await sl.get<Connection>().pair(server);
      await sl.get<NotificationsHook>().onPaired();
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = 'No pude guardar el token en este teléfono. Inténtalo de nuevo.';
      });
      return;
    }
    if (!mounted) return;
    _link.clear();
    setState(() => _busy = false);
    widget.onPaired?.call();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Vincular con tu equipo')),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(20, 24, 20, 24),
          children: <Widget>[
            const Text(
              'En tu equipo abre ComandOS → «Remoto». Escanea su código QR o pega el '
              'enlace que muestra. El teléfono necesita Tailscale encendido.',
              style: TextStyle(color: Orbita.dim, fontSize: 15, height: 1.4),
            ),
            const SizedBox(height: 24),
            FilledButton.icon(
              onPressed: _busy ? null : _scan,
              icon: const Icon(Icons.qr_code_scanner),
              label: const Text('Escanear código QR'),
              style: FilledButton.styleFrom(minimumSize: const Size.fromHeight(56)),
            ),
            const SizedBox(height: 28),
            const Text('O pega el enlace', style: TextStyle(color: Orbita.dim)),
            const SizedBox(height: 8),
            TextField(
              controller: _link,
              enabled: !_busy,
              keyboardType: TextInputType.url,
              autocorrect: false,
              enableSuggestions: false,
              minLines: 1,
              maxLines: 3,
              decoration: const InputDecoration(
                hintText: 'https://mi-equipo.mi-red.ts.net/?token=…',
              ),
              onSubmitted: (_) => _submit(),
            ),
            const SizedBox(height: 12),
            OutlinedButton(
              onPressed: _busy ? null : _submit,
              child: _busy
                  ? const SizedBox(
                      width: 20, height: 20, child: CircularProgressIndicator(strokeWidth: 2))
                  : const Text('Vincular'),
            ),
            if (_error != null) ...<Widget>[
              const SizedBox(height: 16),
              Text(_error!, style: const TextStyle(color: Orbita.err, fontSize: 15, height: 1.35)),
            ],
          ],
        ),
      ),
    );
  }
}
