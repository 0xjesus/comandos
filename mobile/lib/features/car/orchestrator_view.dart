import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'orchestrator_call.dart';

/// Red of the hang-up key (a call's universal colour, darker than `--err`).
const Color kHangUp = Color(0xFFE5484D);

/// What the call screen says in each moment: a big title and a hint of
/// what to do next.
(String, String) callWords(OrchPhase phase, String status, {required bool wake}) => switch (phase) {
      OrchPhase.connecting => ('Conectando', 'Abro la sesión del orquestador en tu computadora.'),
      OrchPhase.sleeping => ('Di «OK ComandOS»', 'O toca Hablar, o el botón del volante.'),
      OrchPhase.listening => ('Te escucho', 'Habla con calma. Termino cuando te quedes callado.'),
      OrchPhase.sending => ('Enviando', 'Le paso lo que dijiste al orquestador.'),
      OrchPhase.thinking => (
          status.startsWith('Entendiendo') ? 'Entendiendo' : 'Pensando',
          status.startsWith('Entendiendo') ? 'Transcribo lo que dijiste.' : 'El orquestador revisa tus sesiones.',
        ),
      OrchPhase.speaking => ('Hablando', 'Toca Callar para interrumpir.'),
      OrchPhase.asking => ('Te pregunta', 'Di el número de la opción.'),
      OrchPhase.idle => ('En espera', wake ? 'Toca Hablar o di «OK ComandOS».' : 'Toca Hablar o el botón del volante.'),
      OrchPhase.failed => ('Algo falló', status),
      OrchPhase.ended => ('Llamada terminada', 'Toca Salir para volver.'),
    };

Color callTone(OrchPhase phase) => switch (phase) {
      OrchPhase.listening => Orbita.done,
      OrchPhase.speaking => Orbita.brand,
      OrchPhase.thinking || OrchPhase.sending || OrchPhase.connecting => Orbita.working,
      OrchPhase.asking => Orbita.waiting,
      OrchPhase.sleeping => Orbita.brand,
      OrchPhase.failed => Orbita.err,
      OrchPhase.idle || OrchPhase.ended => Orbita.faint,
    };

IconData callGlyph(OrchPhase phase) => switch (phase) {
      OrchPhase.listening => Icons.mic_rounded,
      OrchPhase.speaking => Icons.graphic_eq_rounded,
      OrchPhase.thinking || OrchPhase.sending || OrchPhase.connecting => Icons.auto_awesome_rounded,
      OrchPhase.asking => Icons.help_outline_rounded,
      OrchPhase.sleeping => Icons.hearing_rounded,
      OrchPhase.failed => Icons.error_outline_rounded,
      OrchPhase.idle || OrchPhase.ended => Icons.phone_in_talk_rounded,
    };

/// The Modo auto call, drawn like the rest of the app (Noche Órbita): a
/// header, a breathing orb in the colour of the moment, one title and one
/// hint, what was heard and answered last, and two keys (talk, hang up).
/// Pure: [OrchestratorScreen] feeds it the call's state.
class OrchestratorCallView extends StatelessWidget {
  const OrchestratorCallView({
    super.key,
    required this.phase,
    required this.status,
    required this.heard,
    required this.said,
    required this.wake,
    required this.level,
    required this.onTalk,
    required this.onHush,
    required this.onStop,
    required this.onHangUp,
    this.animate = true,
  });

  final OrchPhase phase;
  final String status;
  final String heard;
  final String said;

  /// «OK ComandOS» can be heard on this phone.
  final bool wake;

  /// Microphone level 0..1 while listening.
  final double level;
  final VoidCallback onTalk;
  final VoidCallback onHush;
  final VoidCallback onStop;
  final VoidCallback onHangUp;

  /// Off in tests and previews (a still frame).
  final bool animate;

  @override
  Widget build(BuildContext context) {
    final Color tone = callTone(phase);
    return Scaffold(
      backgroundColor: Orbita.bg,
      body: Stack(
        children: <Widget>[
          Positioned.fill(
            child: TweenAnimationBuilder<Color?>(
              tween: ColorTween(end: tone),
              duration: const Duration(milliseconds: 500),
              builder: (BuildContext context, Color? c, Widget? _) => DecoratedBox(
                decoration: BoxDecoration(
                  gradient: RadialGradient(
                    center: const Alignment(0, -0.38),
                    radius: 0.95,
                    colors: <Color>[(c ?? tone).withValues(alpha: 0.16), Orbita.bg.withValues(alpha: 0)],
                  ),
                ),
              ),
            ),
          ),
          SafeArea(
            child: OrientationBuilder(
              builder: (BuildContext context, Orientation o) {
                final Widget header = _Header(phase: phase, onExit: onHangUp);
                if (o == Orientation.landscape) {
                  return Column(children: <Widget>[
                    header,
                    Expanded(
                      child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: <Widget>[
                        Expanded(flex: 5, child: _centre(context, tone, compact: true)),
                        Expanded(
                          flex: 4,
                          child: Column(mainAxisAlignment: MainAxisAlignment.end, children: <Widget>[
                            Expanded(child: Center(child: _Transcript(heard: heard, said: said))),
                            _keys(),
                          ]),
                        ),
                      ]),
                    ),
                  ]);
                }
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    header,
                    Expanded(child: _centre(context, tone)),
                    _Transcript(heard: heard, said: said),
                    _keys(),
                  ],
                );
              },
            ),
          ),
        ],
      ),
    );
  }

  Widget _centre(BuildContext context, Color tone, {bool compact = false}) {
    final (String title, String hint) = callWords(phase, status, wake: wake);
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints b) {
        final double orb = math.min(b.maxWidth * 0.62, b.maxHeight - (compact ? 120 : 150)).clamp(120, 260).toDouble();
        return Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: <Widget>[
            CallOrb(tone: tone, glyph: callGlyph(phase), size: orb, level: level, moving: _moving, animate: animate),
            SizedBox(height: compact ? 14 : 22),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24),
              child: Text(
                title,
                textAlign: TextAlign.center,
                maxLines: 2,
                style: const TextStyle(
                    color: Orbita.text, fontSize: 30, fontWeight: FontWeight.w700, letterSpacing: -0.4, height: 1.15),
              ),
            ),
            const SizedBox(height: 8),
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 340),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 24),
                child: Text(
                  hint,
                  textAlign: TextAlign.center,
                  maxLines: 3,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: Orbita.dim, fontSize: 15.5, height: 1.4),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  bool get _moving => phase != OrchPhase.idle && phase != OrchPhase.failed && phase != OrchPhase.ended;

  Widget _keys() {
    final bool speaking = phase == OrchPhase.speaking;
    final bool listening = phase == OrchPhase.listening;
    final bool busy = phase == OrchPhase.thinking || phase == OrchPhase.sending;
    final bool ended = phase == OrchPhase.ended;
    final Widget primary = speaking
        ? _Key(label: 'Callar', icon: Icons.stop_rounded, face: Orbita.panel2, ink: Orbita.waiting, onTap: onHush)
        : listening
            ? _Key(label: 'Listo', icon: Icons.check_rounded, face: Orbita.done, ink: Orbita.onBrand, onTap: onTalk)
            : _Key(
                label: 'Hablar',
                icon: Icons.mic_rounded,
                face: Orbita.brand,
                ink: Orbita.onBrand,
                onTap: ended || phase == OrchPhase.connecting ? null : onTalk,
              );
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Row(children: <Widget>[
            Expanded(child: primary),
            if (busy) ...<Widget>[
              const SizedBox(width: 12),
              SizedBox(
                width: 132,
                child: _Key(label: 'Detener', icon: Icons.stop_circle_outlined, face: Orbita.panel2, ink: Orbita.err, onTap: onStop),
              ),
            ],
          ]),
          const SizedBox(height: 12),
          _Key(
            label: ended ? 'Salir' : 'Colgar',
            icon: ended ? Icons.arrow_back_rounded : Icons.call_end_rounded,
            face: ended ? Orbita.panel2 : kHangUp,
            ink: ended ? Orbita.text : Colors.white,
            onTap: onHangUp,
          ),
        ],
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header({required this.phase, required this.onExit});

  final OrchPhase phase;
  final VoidCallback onExit;

  @override
  Widget build(BuildContext context) {
    final (String label, Color dot) = switch (phase) {
      OrchPhase.connecting => ('Conectando', Orbita.working),
      OrchPhase.failed => ('Con problemas', Orbita.err),
      OrchPhase.ended => ('Colgada', Orbita.faint),
      _ => ('En línea', Orbita.done),
    };
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 16, 4),
      child: Row(children: <Widget>[
        WebButton(
          tooltip: 'Salir del Modo auto',
          size: 44,
          radius: 22,
          lip: 2,
          onTap: onExit,
          child: const Icon(Icons.keyboard_arrow_down_rounded, size: 26, color: Orbita.text),
        ),
        const SizedBox(width: 12),
        const Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Text('Modo auto',
                  style: TextStyle(color: Orbita.text, fontSize: 17, fontWeight: FontWeight.w600, letterSpacing: -0.2)),
              SizedBox(height: 2),
              Text('Orquestador · Claude Code', style: TextStyle(color: Orbita.dim, fontSize: 12.5)),
            ],
          ),
        ),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 6),
          decoration: BoxDecoration(
            color: Orbita.panel,
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: Orbita.line),
          ),
          child: Row(mainAxisSize: MainAxisSize.min, children: <Widget>[
            Container(width: 8, height: 8, decoration: BoxDecoration(color: dot, shape: BoxShape.circle)),
            const SizedBox(width: 7),
            Text(label, style: const TextStyle(color: Orbita.dim, fontSize: 12.5, fontWeight: FontWeight.w500)),
          ]),
        ),
      ]),
    );
  }
}

/// What was heard and answered last, small: a glance, not something to read.
class _Transcript extends StatelessWidget {
  const _Transcript({required this.heard, required this.said});

  final String heard;
  final String said;

  @override
  Widget build(BuildContext context) {
    if (heard.trim().isEmpty && said.trim().isEmpty) return const SizedBox.shrink();
    Widget row(String who, String text, int lines, Color ink) => Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text(who,
                style: const TextStyle(
                    color: Orbita.faint, fontSize: 11, fontWeight: FontWeight.w600, letterSpacing: 0.8)),
            const SizedBox(height: 3),
            Text(text.trim(),
                maxLines: lines,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: ink, fontSize: 14.5, height: 1.4)),
          ],
        );
    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 16),
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      decoration: BoxDecoration(
        color: Orbita.panel.withValues(alpha: 0.85),
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: Orbita.line),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          if (heard.trim().isNotEmpty) row('TÚ', heard, 2, Orbita.text),
          if (heard.trim().isNotEmpty && said.trim().isNotEmpty) const SizedBox(height: 10),
          if (said.trim().isNotEmpty) row('COMANDOS', said, 3, Orbita.dim),
        ],
      ),
    );
  }
}

class _Key extends StatelessWidget {
  const _Key({required this.label, required this.icon, required this.face, required this.ink, required this.onTap});

  final String label;
  final IconData icon;
  final Color face;
  final Color ink;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => SizedBox(
        height: 64,
        width: double.infinity,
        child: WebButton(
          tooltip: label,
          onTap: onTap,
          radius: 18,
          lip: 3,
          minHeight: 64,
          face: face,
          ink: ink,
          child: FittedBox(
            fit: BoxFit.scaleDown,
            child: Row(mainAxisSize: MainAxisSize.min, children: <Widget>[
              Icon(icon, size: 24, color: ink),
              const SizedBox(width: 10),
              Text(label, style: TextStyle(color: ink, fontSize: 19, fontWeight: FontWeight.w700)),
            ]),
          ),
        ),
      );
}

/// The call's centre: soft rings that breathe in the colour of the moment,
/// and a disc that swells with the voice while listening.
class CallOrb extends StatefulWidget {
  const CallOrb({
    super.key,
    required this.tone,
    required this.glyph,
    required this.size,
    required this.level,
    required this.moving,
    this.animate = true,
  });

  final Color tone;
  final IconData glyph;
  final double size;
  final double level;
  final bool moving;
  final bool animate;

  @override
  State<CallOrb> createState() => _CallOrbState();
}

class _CallOrbState extends State<CallOrb> with SingleTickerProviderStateMixin {
  late final AnimationController _t = AnimationController(vsync: this, duration: const Duration(milliseconds: 2400));

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync();
  }

  @override
  void didUpdateWidget(CallOrb old) {
    super.didUpdateWidget(old);
    _sync();
  }

  void _sync() {
    final bool run = widget.animate && widget.moving && !(MediaQuery.maybeDisableAnimationsOf(context) ?? false);
    if (run && !_t.isAnimating) {
      _t.repeat();
    } else if (!run && _t.isAnimating) {
      _t.stop();
    }
  }

  @override
  void dispose() {
    _t.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final double s = widget.size;
    final Color tone = widget.tone;
    return RepaintBoundary(
      child: AnimatedBuilder(
        animation: _t,
        builder: (BuildContext context, Widget? _) {
          final double t = widget.moving ? _t.value : 0.25;
          final double swell = 1 + widget.level.clamp(0.0, 1.0) * 0.16;
          return SizedBox.square(
            dimension: s,
            child: Stack(alignment: Alignment.center, children: <Widget>[
              for (int ring = 0; ring < 3; ring++)
                Builder(builder: (BuildContext context) {
                  final double p = (t + ring / 3) % 1;
                  return Container(
                    width: s * (0.52 + 0.48 * p),
                    height: s * (0.52 + 0.48 * p),
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      border: Border.all(
                          color: tone.withValues(alpha: (widget.moving ? (1 - p) * 0.42 : 0.14)), width: 1.5),
                    ),
                  );
                }),
              AnimatedContainer(
                duration: const Duration(milliseconds: 120),
                width: s * 0.46 * swell,
                height: s * 0.46 * swell,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: <Color>[Color.lerp(tone, Colors.white, 0.18)!, Color.lerp(tone, Orbita.bg, 0.35)!],
                  ),
                  border: Border.all(color: Colors.white.withValues(alpha: 0.14)),
                  boxShadow: <BoxShadow>[
                    BoxShadow(color: tone.withValues(alpha: 0.38), blurRadius: s * 0.22, spreadRadius: 2),
                  ],
                ),
                child: Icon(widget.glyph, size: s * 0.17, color: Orbita.onBrand),
              ),
            ]),
          );
        },
      ),
    );
  }
}
