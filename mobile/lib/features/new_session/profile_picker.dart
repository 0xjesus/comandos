import 'package:flutter/material.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/json_read.dart';
import '../../ui/theme.dart';
import 'wizard_api.dart';
import 'wizard_widgets.dart';

/// «Elegir perfil»: the saved session profiles (`GET /session-profiles`) in a
/// `.sc-panel` titled «Perfiles de sesión». Completes with the chosen profile
/// id («Nueva sesión con este perfil») or null.
///
/// The web opens its full profile editor here (skills, MCPs, save); the app
/// only lists and picks saved profiles.
Future<String?> showProfilePicker(
  BuildContext context, {
  required String harness,
  required String account,
  required String cwd,
}) =>
    showGeneralDialog<String>(
      context: context,
      barrierDismissible: true,
      barrierLabel: 'Cerrar',
      barrierColor: const Color(0x88000000),
      transitionDuration: Duration.zero,
      pageBuilder: (BuildContext c, Animation<double> a, Animation<double> b) =>
          ProfilePicker(harness: harness, account: account, cwd: cwd),
    );

class ProfilePicker extends StatefulWidget {
  const ProfilePicker({super.key, required this.harness, required this.account, required this.cwd});

  final String harness;
  final String account;
  final String cwd;

  @override
  State<ProfilePicker> createState() => _ProfilePickerState();
}

class _ProfilePickerState extends State<ProfilePicker> {
  List<(String, String)>? _profiles;
  String? _error;
  String? _chosen;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final ComandosApi? api = sl.maybe<Connection>()?.api;
    if (api == null) return;
    try {
      final Object? r = await api.getJson('/session-profiles', query: <String, String>{
        'harness': widget.harness,
        'account': widget.account,
        if (widget.cwd.isNotEmpty) 'cwd': widget.cwd,
      });
      final Map<String, Object?> m = asMap(r) ?? const <String, Object?>{};
      if (!mounted) return;
      setState(() => _profiles = <(String, String)>[
            for (final Map<String, Object?> p in readMapList(m, 'profiles'))
              if (readString(p, 'id').isNotEmpty) (readString(p, 'id'), readString(p, 'name', readString(p, 'id'))),
          ]);
    } on Object catch (e) {
      if (mounted) setState(() => _error = errorText(e));
    }
  }

  @override
  Widget build(BuildContext context) {
    final Size screen = MediaQuery.sizeOf(context);
    final List<(String, String)>? list = _profiles;
    final Widget body;
    if (_error != null) {
      body = Text(_error!, style: const TextStyle(fontSize: 13, color: Orbita.waiting));
    } else if (list == null) {
      body = const Text('Cargando…', style: TextStyle(fontSize: 13, color: Orbita.dim));
    } else {
      body = Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          const Text(
            'Guarda CLI, modelo, cuentas, skills y MCPs para iniciar con el mismo equipo. '
            'Editar un perfil no modifica las sesiones abiertas.',
            style: TextStyle(fontSize: 13, height: 1.5, color: Orbita.dim),
          ),
          const SizedBox(height: 12),
          if (list.isEmpty)
            const Text('Sin perfiles guardados', style: TextStyle(fontSize: 13, color: Orbita.faint))
          else
            for (final (String id, String name) in list)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: GestureDetector(
                  key: ValueKey<String>('profile-$id'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => setState(() => _chosen = id),
                  child: Container(
                    constraints: const BoxConstraints(minHeight: 44),
                    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                    alignment: Alignment.centerLeft,
                    decoration: BoxDecoration(
                      color: Orbita.panel2,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: _chosen == id ? Orbita.brand : Orbita.line),
                    ),
                    child: Text(name, style: const TextStyle(fontSize: 13, color: Orbita.text)),
                  ),
                ),
              ),
          const SizedBox(height: 6),
          Align(
            alignment: Alignment.centerRight,
            child: NsActionButton(
              key: const ValueKey<String>('profile-launch'),
              label: 'Nueva sesión con este perfil',
              onTap: _chosen == null ? null : () => Navigator.of(context).pop(_chosen),
            ),
          ),
        ],
      );
    }
    return Center(
      child: Material(
        type: MaterialType.transparency,
        child: Container(
          width: screen.width - 16,
          constraints: BoxConstraints(maxWidth: 760, maxHeight: screen.height * .94),
          decoration: BoxDecoration(
            color: Orbita.panel,
            border: Border.all(color: Orbita.line),
            borderRadius: BorderRadius.circular(12),
          ),
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(14),
            child: DefaultTextStyle(
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.text),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: <Widget>[
                  Row(
                    children: <Widget>[
                      const Expanded(
                        child: Text('Perfiles de sesión', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w700)),
                      ),
                      NsActionButton(label: '×', onTap: () => Navigator.of(context).pop()),
                    ],
                  ),
                  const SizedBox(height: 14),
                  body,
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
