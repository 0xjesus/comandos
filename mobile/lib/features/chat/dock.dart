import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'pane_rows.dart';

/// The keys of `#cv-dock .cv-keys`, in the web's order (`chat_view.rs` `BARS`).
enum DockKey {
  escape,
  ctrlc,
  select,
  panes,
  backspace,
  history,
  copy,
  paste,
  keyboard,
  ctrl,
  tab,
  left,
  up,
  down,
  right,
  one,
  two,
}

/// Label of each key as the web writes it (arrows are icons).
String dockKeyLabel(DockKey k) => switch (k) {
      DockKey.escape => 'Esc',
      DockKey.ctrlc => 'Ctrl+C',
      DockKey.select => 'Seleccionar',
      DockKey.panes => 'Paneles',
      DockKey.backspace => '⌫',
      DockKey.history => 'Historial',
      DockKey.copy => 'Copiar',
      DockKey.paste => 'Pegar',
      DockKey.keyboard => 'Teclado directo',
      DockKey.ctrl => 'Ctrl',
      DockKey.tab => 'Tab',
      DockKey.left => 'Izquierda',
      DockKey.up => 'Arriba',
      DockKey.down => 'Abajo',
      DockKey.right => 'Derecha',
      DockKey.one => '1',
      DockKey.two => '2',
    };

const Map<DockKey, WebIconData> _arrows = <DockKey, WebIconData>{
  DockKey.left: WebIcons.chevronLeft,
  DockKey.up: WebIcons.chevronUp,
  DockKey.down: WebIcons.chevronDown,
  DockKey.right: WebIcons.chevronRight,
};

/// One `.cv-keys button`: 36 px high, ≥44 px wide, `--bg` face, radius 10,
/// 600 13 px mono. [on]: a toggle that is active (Ctrl armed, Teclado directo).
class DockKeyButton extends StatelessWidget {
  const DockKeyButton({super.key, required this.label, required this.onTap, this.icon, this.on = false, this.tooltip});

  final String label;
  final VoidCallback onTap;
  final WebIconData? icon;
  final bool on;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final Color ink = on ? Colors.white : Orbita.text;
    return WebButton(
      onTap: onTap,
      tooltip: tooltip ?? (icon != null ? label : null),
      on: on,
      face: on ? Orbita.brand : Orbita.bg,
      ink: ink,
      radius: 10,
      lip: 2,
      minHeight: 36,
      padding: const EdgeInsets.symmetric(horizontal: 12),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minWidth: 20, minHeight: 36),
        child: Center(
          widthFactor: 1,
          child: icon != null
              ? WebIcon(icon!, size: 18, color: ink, semanticLabel: label)
              : Text(
                  label,
                  maxLines: 1,
                  softWrap: false,
                  style: TextStyle(fontFamily: Orbita.mono, fontSize: 13, fontWeight: FontWeight.w600, color: ink),
                ),
        ),
      ),
    );
  }
}

/// The horizontally scrolling key row. [extra]: the phone's own keys, after
/// the web's (read new answers aloud, Modo auto).
class DockKeys extends StatelessWidget {
  const DockKeys({super.key, required this.onKey, this.ctrlArmed = false, this.direct = false, this.extra = const <Widget>[]});

  final void Function(DockKey key) onKey;
  final bool ctrlArmed;
  final bool direct;
  final List<Widget> extra;

  @override
  Widget build(BuildContext context) {
    final List<Widget> keys = <Widget>[
      for (final DockKey k in DockKey.values)
        DockKeyButton(
          key: ValueKey<DockKey>(k),
          label: dockKeyLabel(k),
          icon: _arrows[k],
          on: (k == DockKey.ctrl && ctrlArmed) || (k == DockKey.keyboard && direct),
          tooltip: switch (k) {
            DockKey.ctrlc => 'Interrumpir (Ctrl+C)',
            DockKey.backspace => 'Borrar un carácter (Backspace)',
            DockKey.keyboard => 'Escribir directo en la terminal',
            _ => null,
          },
          onTap: () => onKey(k),
        ),
      ...extra,
    ];
    return SizedBox(
      height: 50,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        itemCount: keys.length,
        separatorBuilder: (BuildContext context, int i) => const SizedBox(width: 6),
        itemBuilder: (BuildContext context, int i) => keys[i],
      ),
    );
  }
}

/// `.cv-panes`: one big chip per pane of a split (number, agent · model, state
/// dot); the one this phone targets is brand-filled. Hidden without a split.
class PaneChips extends StatelessWidget {
  const PaneChips({super.key, required this.rows, required this.pane, required this.onSelect});

  final List<PaneRow> rows;
  final String pane;
  final void Function(String pane) onSelect;

  @override
  Widget build(BuildContext context) {
    if (rows.length < 2) return const SizedBox.shrink();
    return SizedBox(
      height: 56,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 2),
        itemCount: rows.length,
        separatorBuilder: (BuildContext context, int i) => const SizedBox(width: 6),
        itemBuilder: (BuildContext context, int i) {
          final PaneRow r = rows[i];
          return PaneChip(
            key: ValueKey<String>(r.pane),
            number: i + 1,
            label: paneLabel(r),
            status: r.status,
            on: r.pane == pane,
            onTap: () => onSelect(r.pane),
          );
        },
      ),
    );
  }
}

class PaneChip extends StatelessWidget {
  const PaneChip({super.key, required this.number, required this.label, required this.status, required this.on, required this.onTap});

  final int number;
  final String label;
  final String status;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final Color dot = switch (status) {
      'waiting' => Orbita.waiting,
      'working' => Orbita.working,
      'done' => Orbita.done,
      _ => Orbita.line2,
    };
    return Semantics(
      selected: on,
      child: WebButton(
        onTap: onTap,
        on: on,
        face: on ? Orbita.brand : Orbita.bg,
        ink: on ? Colors.white : Orbita.dim,
        radius: 999,
        lip: 2,
        minHeight: 44,
        padding: const EdgeInsets.symmetric(horizontal: 15),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Container(
              width: 9,
              height: 9,
              decoration: BoxDecoration(
                color: dot,
                shape: BoxShape.circle,
                boxShadow: on ? const <BoxShadow>[BoxShadow(color: Colors.white, spreadRadius: 2)] : null,
              ),
            ),
            const SizedBox(width: 7),
            Text('$number',
                style: TextStyle(fontFamily: Orbita.mono, fontSize: 12, fontWeight: FontWeight.w700, color: on ? Colors.white : Orbita.faint)),
            const SizedBox(width: 7),
            Text(label,
                maxLines: 1,
                softWrap: false,
                style: TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600, color: on ? Colors.white : Orbita.dim)),
          ],
        ),
      ),
    );
  }
}
