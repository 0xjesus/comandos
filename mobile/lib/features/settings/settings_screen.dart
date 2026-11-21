import '../webpanel/web_panel.dart';
import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/config/paired_server.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/outbox/outbox.dart';
import '../../ui/format.dart';
import '../../ui/theme.dart';
import '../car/car_mode.dart';
import '../notifications/notifications_controller.dart';
import '../notifications/notifications_hook.dart';
import '../notifications/notifications_prompt.dart';
import '../pairing/pairing_screen.dart';
import '../voice/voice_settings_section.dart';

/// Pairing management and a few facts about the app.
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  static const String routeName = '/settings';

  Future<void> _repair(BuildContext context) async {
    await Navigator.of(context).push<void>(MaterialPageRoute<void>(
      settings: const RouteSettings(name: PairingScreen.routeName),
      builder: (BuildContext c) => PairingScreen(onPaired: AppNavigator.showSessions),
    ));
  }

  Future<void> _unpair(BuildContext context) async {
    final Outbox outbox = sl.get<Outbox>();
    final int unsettled = outbox.pendingCount + outbox.attentionCount;
    final bool? ok = await showDialog<bool>(
      context: context,
      builder: (BuildContext c) => AlertDialog(
        backgroundColor: Orbita.panel,
        title: const Text('¿Desvincular este teléfono?'),
        content: Text(
          'Se borra el token de este teléfono. Para volver a usarlo tendrás que '
          'escanear el código de «Remoto» otra vez.'
          '${unsettled > 0 ? '\n\nTienes ${plural(unsettled, 'mensaje sin entregar', 'mensajes sin entregar')}: '
              'se guardan y se enviarán cuando vuelvas a vincular este mismo equipo.' : ''}',
        ),
        actions: <Widget>[
          TextButton(onPressed: () => Navigator.of(c).pop(false), child: const Text('Cancelar')),
          TextButton(
            onPressed: () => Navigator.of(c).pop(true),
            style: TextButton.styleFrom(foregroundColor: Orbita.err),
            child: const Text('Desvincular'),
          ),
        ],
      ),
    );
    if (ok != true) return;
    await sl.get<NotificationsHook>().onUnpaired();
    await sl.get<Connection>().unpair();
    await clearWebData();
    AppNavigator.showPairing();
  }

  @override
  Widget build(BuildContext context) {
    final Connection connection = sl.get<Connection>();
    final PairedServer? server = connection.server;
    final NotificationsController? notices = sl.maybe<NotificationsController>();
    return Scaffold(
      appBar: AppBar(title: const Text('Ajustes')),
      body: SafeArea(
        top: false,
        child: ListView(
          children: <Widget>[
            ListTile(
              leading: const Icon(Icons.computer_outlined),
              title: const Text('Equipo vinculado'),
              subtitle: Text(server?.displayHost ?? 'Ninguno'),
            ),
            ListTile(
              leading: const Icon(Icons.qr_code_scanner),
              title: const Text('Volver a vincular'),
              subtitle: const Text('Escanea un código nuevo (por ejemplo, si cambió el token).'),
              onTap: () => _repair(context),
            ),
            if (server != null)
              ListTile(
                leading: const Icon(Icons.link_off, color: Orbita.err),
                title: const Text('Desvincular'),
                subtitle: const Text('Borra el token de este teléfono.'),
                onTap: () => _unpair(context),
              ),
            const Divider(),
            if (notices != null && server != null) ...<Widget>[
              NotificationSettings(controller: notices),
              const Divider(),
            ],
            ListTile(
              leading: const Icon(Icons.directions_car_outlined),
              title: const Text('Modo auto'),
              onTap: () => CarMode.open(context),
            ),
            ListenableBuilder(
              listenable: sl.get<Outbox>(),
              builder: (BuildContext context, Widget? _) {
                final Outbox o = sl.get<Outbox>();
                final int n = o.pendingCount + o.attentionCount;
                return ListTile(
                  leading: const Icon(Icons.outbox_outlined),
                  title: const Text('Mensajes sin entregar'),
                  subtitle: Text(n == 0 ? 'Ninguno' : plural(n, 'mensaje', 'mensajes')),
                  onTap: n == 0 || o.firstUnsettledSession == null
                      ? null
                      : () => AppNavigator.openChat(o.firstUnsettledSession!),
                );
              },
            ),
            const VoiceSettingsSection(),
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 24, 16, 24),
              child: Text(
                'El token se guarda cifrado en el almacén seguro de Android y solo '
                'viaja a tu equipo por Tailscale.',
                style: TextStyle(color: Orbita.faint, fontSize: 13),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
