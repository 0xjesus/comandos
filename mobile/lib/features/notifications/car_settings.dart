import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import 'notifications_controller.dart';
import 'notifications_prompt.dart';

/// «En el coche · Android Auto» in Ajustes: how a notice sounds in the car,
/// whether it reads what the agent wrote, a test, and the one-time setup.
class CarSettings extends StatelessWidget {
  const CarSettings({super.key, required this.controller});

  final NotificationsController controller;

  Future<void> _test() async {
    final String? problem = await controller.testInCar();
    showNoticeMessage(problem ?? 'Aviso de prueba enviado. Contéstalo desde el coche o desde la notificación.',
        openNotificationSettings: problem != null);
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (BuildContext context, Widget? _) {
        final bool content = controller.carContent;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 20, 16, 4),
              child: Text('En el coche · Android Auto',
                  style: TextStyle(color: Orbita.brand, fontWeight: FontWeight.w600)),
            ),
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 0, 16, 12),
              child: Text(
                'Tus sesiones llegan al coche como conversaciones. El coche no lee nada hasta que tú '
                'lo pides, y nunca envía sin repetirte el mensaje completo.',
                style: TextStyle(color: Orbita.dim, fontSize: 13, height: 1.35),
              ),
            ),
            const _Flow(),
            SwitchListTile(
              secondary: Icon(content ? Icons.record_voice_over_outlined : Icons.visibility_off_outlined),
              title: const Text('Leer lo que escribió el agente'),
              subtitle: Text(content
                  ? 'Un resumen de unos 20 segundos, sin código ni rutas. Llaves, tokens, correos, '
                      'teléfonos e IPs se ocultan siempre.'
                  : 'El coche solo dice qué sesión terminó o te espera. Los permisos siguen leyendo '
                      'el comando: sin él no podrías aprobar con seguridad.'),
              value: content,
              onChanged: controller.setCarContent,
            ),
            ListTile(
              leading: const Icon(Icons.campaign_outlined),
              title: const Text('Probar en el coche'),
              subtitle: const Text('Manda un aviso de prueba. Contéstalo con la voz: no llega a ningún agente.'),
              onTap: _test,
            ),
            const _Setup(),
          ],
        );
      },
    );
  }
}

/// The three moments of a car notice, as a compact strip.
class _Flow extends StatelessWidget {
  const _Flow();

  @override
  Widget build(BuildContext context) {
    const List<(IconData, String, String)> steps = <(IconData, String, String)>[
      (Icons.notifications_active_outlined, 'Te llega', 'Terminó, te espera o falló'),
      (Icons.mic_none_outlined, 'Lo escuchas', 'Botón de voz del volante o toca el aviso'),
      (Icons.verified_outlined, 'Respondes', 'Te lo repite y pregunta «¿Lo envío?»'),
    ];
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 8),
        decoration: BoxDecoration(
          color: Orbita.panel,
          border: Border.all(color: Orbita.line),
          borderRadius: BorderRadius.circular(14),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            for (final (int i, (IconData, String, String) s) in steps.indexed) ...<Widget>[
              if (i > 0)
                const Padding(
                  padding: EdgeInsets.only(top: 10),
                  child: Icon(Icons.chevron_right, size: 18, color: Orbita.faint),
                ),
              Expanded(
                child: Column(
                  children: <Widget>[
                    Icon(s.$1, color: Orbita.brand, size: 26),
                    const SizedBox(height: 6),
                    Text(s.$2,
                        textAlign: TextAlign.center,
                        style: const TextStyle(color: Orbita.text, fontWeight: FontWeight.w600, fontSize: 13)),
                    const SizedBox(height: 2),
                    Text(s.$3,
                        textAlign: TextAlign.center,
                        style: const TextStyle(color: Orbita.dim, fontSize: 11.5, height: 1.3)),
                  ],
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// One-time setup: ComandOS is not from Play, so Android Auto must be told to
/// show apps installed by hand.
class _Setup extends StatelessWidget {
  const _Setup();

  @override
  Widget build(BuildContext context) {
    const List<String> steps = <String>[
      'En el teléfono abre Android Auto (Ajustes › Dispositivos conectados › Preferencias de conexión › Android Auto).',
      'Baja hasta «Versión» y tócala 10 veces: se activa el modo de desarrollador.',
      'Menú ⋮ › «Configuración para desarrolladores» › activa «Fuentes desconocidas».',
      'De vuelta en Android Auto › Mensajes: activa «Mostrar notificaciones de mensajes».',
      'Toca «Probar en el coche» con el teléfono conectado: debe aparecer en la pantalla del coche.',
    ];
    return Theme(
      data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
      child: ExpansionTile(
        leading: const Icon(Icons.build_circle_outlined),
        title: const Text('Activarlo en Android Auto (una sola vez)'),
        subtitle: const Text('ComandOS no viene de Play: hay que permitirlo.'),
        childrenPadding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
        children: <Widget>[
          for (final (int i, String s) in steps.indexed)
            Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Container(
                    width: 22,
                    height: 22,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: Orbita.panel2,
                      border: Border.all(color: Orbita.line2),
                      borderRadius: BorderRadius.circular(11),
                    ),
                    child: Text('${i + 1}',
                        style: const TextStyle(color: Orbita.dim, fontSize: 11.5, fontWeight: FontWeight.w600)),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(s, style: const TextStyle(color: Orbita.text, fontSize: 13.5, height: 1.35)),
                  ),
                ],
              ),
            ),
          const Text(
            'Para responder di «Responder» (o toca el aviso), dicta tu mensaje y confirma con «Sí». '
            'Un permiso se contesta con el número de la opción; si cambió mientras tanto, no se responde nada.',
            style: TextStyle(color: Orbita.dim, fontSize: 12.5, height: 1.35),
          ),
        ],
      ),
    );
  }
}
