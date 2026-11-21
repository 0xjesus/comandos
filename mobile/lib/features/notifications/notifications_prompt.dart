import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/locator.dart';
import '../../ui/theme.dart';
import 'notifications_controller.dart';

/// The one-time question right after pairing. Null when no screen is up.
Future<bool?> askBackgroundNotices() async {
  if (!AppNavigator.homeShown.value) return null;
  final BuildContext? context = AppNavigator.key.currentState?.overlay?.context;
  if (context == null || !context.mounted) return null;
  return showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (BuildContext c) => AlertDialog(
      backgroundColor: Orbita.panel,
      title: const Text('¿Te aviso cuando una sesión te necesite?'),
      content: const Text(
        'Si lo activas, ComandOS queda atento en segundo plano y te avisa cuando un '
        'agente te espera, termina o falla, aunque la app esté cerrada.\n\n'
        'Verás un aviso fijo y discreto mientras está activo. Gasta poca batería y lo '
        'puedes apagar en Ajustes.',
      ),
      actions: <Widget>[
        TextButton(onPressed: () => Navigator.of(c).pop(false), child: const Text('Ahora no')),
        FilledButton(onPressed: () => Navigator.of(c).pop(true), child: const Text('Sí, avisarme')),
      ],
    ),
  );
}

/// A short message at the bottom of the current screen.
void showNoticeMessage(String message, {bool openNotificationSettings = false}) {
  final BuildContext? context = AppNavigator.key.currentState?.overlay?.context;
  if (context == null || !context.mounted) return;
  final ScaffoldMessengerState? m = ScaffoldMessenger.maybeOf(context);
  if (m == null) return;
  m
    ..hideCurrentSnackBar()
    ..showSnackBar(SnackBar(
      content: Text(message),
      action: openNotificationSettings
          ? SnackBarAction(
              label: 'Abrir ajustes',
              onPressed: () =>
                  sl.maybe<NotificationsController>()?.openSettings(notifications: true),
            )
          : null,
    ));
}

/// The notification rows of Ajustes.
class NotificationSettings extends StatelessWidget {
  const NotificationSettings({super.key, required this.controller});

  final NotificationsController controller;

  Future<void> _toggle(BuildContext context, bool on) async {
    final String? problem = await controller.setEnabled(on);
    if (problem != null) showNoticeMessage(problem, openNotificationSettings: !controller.allowed);
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (BuildContext context, Widget? _) {
        final bool on = controller.enabled;
        final String subtitle;
        if (on && !controller.allowed) {
          subtitle = 'Android no deja mostrar notificaciones. Toca «Permitir notificaciones».';
        } else if (on) {
          subtitle = 'Te aviso cuando una sesión te espera, termina o falla, aunque la app '
              'esté cerrada. Verás un aviso fijo mientras está activo.';
        } else {
          subtitle = 'Apagado. Solo ves novedades con la app abierta.';
        }
        return Column(
          children: <Widget>[
            SwitchListTile(
              secondary: const Icon(Icons.notifications_outlined),
              title: const Text('Avisos en segundo plano'),
              subtitle: Text(subtitle),
              value: on,
              onChanged: (bool v) => _toggle(context, v),
            ),
            if (on && !controller.allowed)
              ListTile(
                leading: const Icon(Icons.notifications_off_outlined, color: Orbita.waiting),
                title: const Text('Permitir notificaciones'),
                subtitle: const Text('Abre los ajustes de Android para esta app.'),
                onTap: () => controller.openSettings(notifications: true),
              ),
            if (on)
              SwitchListTile(
                secondary: const Icon(Icons.done_all),
                title: const Text('Avisar también cuando terminan'),
                subtitle: const Text('Si lo apagas, solo te aviso cuando te esperan o algo falla.'),
                value: controller.alsoDone,
                onChanged: controller.setAlsoDone,
              ),
            if (on)
              ListTile(
                leading: const Icon(Icons.battery_saver_outlined),
                title: const Text('¿Los avisos llegan tarde?'),
                subtitle: const Text('En los ajustes de la app, pon la batería en «Sin restricciones».'),
                onTap: () => controller.openSettings(),
              ),
          ],
        );
      },
    );
  }
}
