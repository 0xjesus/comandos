import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/models/transcript.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../voice/chat_voice.dart';
import 'markdown.dart';

/// Parsed markdown per message, kept on the message object (messages are
/// reused across polls while their content is equal).
class _Rendered {
  _Rendered(this.blocks);
  final List<MdBlock> blocks;
  String? plain;
}

_Rendered _render(ChatMessage m) {
  final Object? cache = m.renderCache;
  if (cache is _Rendered) return cache;
  final _Rendered r = _Rendered(parseMarkdown(m.text));
  m.renderCache = r;
  return r;
}

/// Clean text of a message for the clipboard (no `**`, bullets as «•»).
String plainOf(ChatMessage m) {
  final _Rendered r = _render(m);
  return r.plain ??= markdownPlain(r.blocks);
}

/// «✎ Editar» on an answer: its text becomes editable right in its card, only
/// here (the agent never sees it), to fix it before copying (`Chat::edit`).
/// One card at a time; «Listo» keeps the change («Editado solo aquí», «Ver
/// original»), «Deshacer» goes back to the agent's text.
class BubbleEdits extends ChangeNotifier {
  ChatMessage? _editing;
  final Expando<String> _edited = Expando<String>('edited');

  /// Reads the open editor (set by the card while it edits).
  String Function()? _reader;

  ChatMessage? get editing => _editing;

  /// What the open editor holds now, else the kept edit, else the plain text.
  String currentText(ChatMessage m) =>
      (identical(_editing, m) ? _reader?.call() : null) ?? _edited[m] ?? plainOf(m);

  /// Closes an open editor keeping its changes («Listo»), if any is open.
  void finishOpen() {
    final ChatMessage? m = _editing;
    if (m != null) finish(m, currentText(m));
  }
  bool isEditing(ChatMessage m) => identical(_editing, m);
  String? editedText(ChatMessage m) => _edited[m];

  void start(ChatMessage m) {
    if (identical(_editing, m)) return;
    _editing = m;
    notifyListeners();
  }

  /// Leaves edit mode keeping [text] (when it differs from the original).
  void finish(ChatMessage m, String text) {
    if (text != plainOf(m)) _edited[m] = text;
    if (identical(_editing, m)) _editing = null;
    notifyListeners();
  }

  /// Back to the agent's text.
  void restore(ChatMessage m) {
    _edited[m] = null;
    if (identical(_editing, m)) _editing = null;
    notifyListeners();
  }
}

/// One transcript message as `chat.rs` `bubble()` + `.cv-*` CSS draw it:
/// the agent's answer card, the person's brand bubble, or slim tool lines.
class MessageView extends StatelessWidget {
  const MessageView(this.message, {super.key, required this.edits});

  final ChatMessage message;
  final BubbleEdits edits;

  @override
  Widget build(BuildContext context) {
    switch (message.role) {
      case ChatRole.user:
        return _MeBubble(message);
      case ChatRole.assistant:
        return _AiCard(message, edits: edits);
      case ChatRole.tool:
        return _ToolLines(message);
    }
  }
}

/// `.cv-act`: 44 px raised button under a bubble (`--bg` face, radius 12,
/// 600 13.5 px). [me]: the translucent variant inside the brand bubble.
class CvAct extends StatelessWidget {
  const CvAct({super.key, required this.label, required this.onTap, this.me = false, this.primary = false, this.ok = false, this.icon});

  final String label;
  final VoidCallback? onTap;
  final bool me;
  final bool primary;
  final bool ok;
  final WebIconData? icon;

  @override
  Widget build(BuildContext context) {
    final Color ink = ok
        ? Orbita.done
        : primary
            ? Orbita.onBrand
            : me
                ? Colors.white
                : Orbita.text;
    final Widget text = Text(label, style: TextStyle(fontFamily: Orbita.sans, fontSize: 13.5, fontWeight: FontWeight.w600, color: ink));
    return SelectionContainer.disabled(
      child: WebButton(
        onTap: onTap,
        face: primary ? Orbita.brand : (me ? Colors.black.withValues(alpha: .18) : Orbita.bg),
        ink: ink,
        on: ok,
        radius: 12,
        lip: 2,
        minHeight: me ? 36 : 44,
        padding: const EdgeInsets.symmetric(horizontal: 14),
        child: icon == null
            ? text
            : Row(mainAxisSize: MainAxisSize.min, children: <Widget>[WebIcon(icon!, size: 16, color: ink), const SizedBox(width: 6), text]),
      ),
    );
  }
}

/// A [CvAct] that copies and says «Copiado ✓» for 1.6 s.
class CopyAct extends StatefulWidget {
  const CopyAct({super.key, required this.text, this.label = 'Copiar', this.me = false, this.primary = false});

  final String Function() text;
  final String label;
  final bool me;
  final bool primary;

  @override
  State<CopyAct> createState() => _CopyActState();
}

class _CopyActState extends State<CopyAct> {
  bool _ok = false;
  Timer? _t;

  @override
  void dispose() {
    _t?.cancel();
    super.dispose();
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.text()));
    if (!mounted) return;
    setState(() => _ok = true);
    _t?.cancel();
    _t = Timer(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _ok = false);
    });
  }

  @override
  Widget build(BuildContext context) => CvAct(
        label: _ok ? 'Copiado ✓' : widget.label,
        onTap: _copy,
        me: widget.me,
        primary: widget.primary && !_ok,
        ok: _ok,
      );
}

class _MeBubble extends StatelessWidget {
  const _MeBubble(this.message);
  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    final List<MdBlock> blocks = _render(message).blocks;
    return Align(
      alignment: Alignment.centerRight,
      child: FractionallySizedBox(
        widthFactor: .85,
        alignment: Alignment.centerRight,
        child: Align(
          alignment: Alignment.centerRight,
          child: IntrinsicWidth(
            child: Container(
            padding: const EdgeInsets.fromLTRB(14, 10, 14, 6),
            decoration: const BoxDecoration(
              color: Orbita.brand,
              borderRadius: BorderRadius.only(
                topLeft: Radius.circular(18),
                topRight: Radius.circular(18),
                bottomLeft: Radius.circular(18),
                bottomRight: Radius.circular(4),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                MarkdownView(blocks, fontSize: 14, lineHeight: 1.5, me: true),
                const SizedBox(height: 8),
                CopyAct(text: () => plainOf(message), me: true),
              ],
            ),
          ),
          ),
        ),
      ),
    );
  }
}

class _AiCard extends StatelessWidget {
  const _AiCard(this.message, {required this.edits});
  final ChatMessage message;
  final BubbleEdits edits;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: edits,
      builder: (BuildContext context, Widget? _) {
        final bool editing = edits.isEditing(message);
        final String? edited = edits.editedText(message);
        final Widget body = editing
            ? _EditBody(message: message, edits: edits, initial: edited ?? plainOf(message))
            : edited != null
                ? Text(edited, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 15, height: 1.6, color: Orbita.text))
                : MarkdownView(_render(message).blocks);
        return Container(
          padding: const EdgeInsets.fromLTRB(15, 12, 15, 8),
          decoration: BoxDecoration(
            color: Orbita.panel,
            border: Border.all(color: editing ? Orbita.brand : Orbita.line),
            borderRadius: const BorderRadius.only(
              topLeft: Radius.circular(18),
              topRight: Radius.circular(18),
              bottomRight: Radius.circular(18),
              bottomLeft: Radius.circular(4),
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              body,
              const SizedBox(height: 8),
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: <Widget>[
                  if (editing)
                    CopyAct(text: () => edits.currentText(message), label: 'Copiar editado', primary: true)
                  else
                    CopyAct(text: () => edited ?? plainOf(message)),
                  if (!editing) CvAct(label: '✎ Editar', onTap: () => edits.start(message)),
                  if (editing)
                    CvAct(label: 'Listo', onTap: () => edits.finish(message, edits.currentText(message))),
                  if (editing || edited != null)
                    CvAct(label: editing ? 'Deshacer' : 'Ver original', onTap: () => edits.restore(message)),
                  ReadAloudAct(text: message.text),
                ],
              ),
              if (edited != null && !editing)
                const Padding(
                  padding: EdgeInsets.only(top: 8),
                  child: Text('Editado solo aquí',
                      style: TextStyle(fontFamily: Orbita.sans, fontSize: 11, fontWeight: FontWeight.w600, color: Orbita.faint)),
                ),
            ],
          ),
        );
      },
    );
  }
}

/// The answer's text, editable in place (dashed brand outline on the web).
class _EditBody extends StatefulWidget {
  const _EditBody({required this.message, required this.edits, required this.initial});
  final ChatMessage message;
  final BubbleEdits edits;
  final String initial;

  @override
  State<_EditBody> createState() => _EditBodyState();
}

class _EditBodyState extends State<_EditBody> {
  late final TextEditingController _c = TextEditingController(text: widget.initial);

  String _read() => _c.text;

  @override
  void initState() {
    super.initState();
    widget.edits._reader = _read;
  }

  @override
  void dispose() {
    if (widget.edits._reader == _read) widget.edits._reader = null;
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(5),
      decoration: BoxDecoration(
        border: Border.all(color: Orbita.brand.withValues(alpha: .7), width: 2),
        borderRadius: BorderRadius.circular(4),
      ),
      child: TextField(
        controller: _c,
        autofocus: true,
        maxLines: null,
        keyboardType: TextInputType.multiline,
        cursorColor: Orbita.brand,
        style: const TextStyle(fontFamily: Orbita.sans, fontSize: 15, height: 1.6, color: Orbita.text),
        decoration: const InputDecoration(
          isCollapsed: true,
          filled: false,
          border: InputBorder.none,
          enabledBorder: InputBorder.none,
          focusedBorder: InputBorder.none,
        ),
      ),
    );
  }
}

/// `.cv-tool`: one slim mono line per tool call, under a 2 px rule.
class _ToolLines extends StatelessWidget {
  const _ToolLines(this.message);
  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    final List<String> lines = message.text.split('\n');
    return Align(
      alignment: Alignment.centerLeft,
      child: Container(
        padding: const EdgeInsets.fromLTRB(10, 2, 4, 2),
        decoration: const BoxDecoration(border: Border(left: BorderSide(color: Orbita.line2, width: 2))),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            for (final (int i, String l) in lines.indexed) ...<Widget>[
              if (i > 0) const SizedBox(height: 2),
              Text(
                l,
                maxLines: 1,
                softWrap: false,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11.5, color: Orbita.faint, height: 1.3),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// `.cv-typing`: the agent is on it (three bouncing dots in a card).
class TypingBubble extends StatefulWidget {
  const TypingBubble({super.key});

  @override
  State<TypingBubble> createState() => _TypingBubbleState();
}

class _TypingBubbleState extends State<TypingBubble> with SingleTickerProviderStateMixin {
  late final AnimationController _a = AnimationController(vsync: this, duration: const Duration(milliseconds: 1200))..repeat();

  @override
  void dispose() {
    _a.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.centerLeft,
      child: Semantics(
        label: 'El agente está trabajando',
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 13),
          decoration: BoxDecoration(
            color: Orbita.panel,
            border: Border.all(color: Orbita.line),
            borderRadius: const BorderRadius.only(
              topLeft: Radius.circular(18),
              topRight: Radius.circular(18),
              bottomRight: Radius.circular(18),
              bottomLeft: Radius.circular(4),
            ),
          ),
          child: AnimatedBuilder(
            animation: _a,
            builder: (BuildContext context, Widget? _) => Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                for (int i = 0; i < 3; i++) ...<Widget>[
                  if (i > 0) const SizedBox(width: 5),
                  _dot((_a.value - i * .125) % 1.0),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// `@keyframes cv-dot`: 0/80/100 % rest at .35, 40 % up 4 px at full.
  Widget _dot(double t) {
    final double k = t < .4 ? t / .4 : (t < .8 ? (.8 - t) / .4 : 0);
    final double e = Curves.easeInOut.transform(k.clamp(0, 1));
    return Transform.translate(
      offset: Offset(0, -4 * e),
      child: Opacity(
        opacity: .35 + .65 * e,
        child: Container(width: 7, height: 7, decoration: const BoxDecoration(color: Orbita.dim, shape: BoxShape.circle)),
      ),
    );
  }
}

/// `.cv-skel`: three pulsing placeholders while the first answer loads.
class ChatSkeleton extends StatefulWidget {
  const ChatSkeleton({super.key});

  @override
  State<ChatSkeleton> createState() => _ChatSkeletonState();
}

class _ChatSkeletonState extends State<ChatSkeleton> with SingleTickerProviderStateMixin {
  late final AnimationController _a = AnimationController(vsync: this, duration: const Duration(milliseconds: 900))
    ..repeat(reverse: true);

  @override
  void dispose() {
    _a.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    Widget bar(double factor, double height, Alignment align, double delay) => Align(
          alignment: align,
          child: FractionallySizedBox(
            widthFactor: factor,
            child: AnimatedBuilder(
              animation: _a,
              builder: (BuildContext context, Widget? child) =>
                  Opacity(opacity: .35 + .45 * ((_a.value + delay) % 1.0), child: child),
              child: Container(
                height: height,
                decoration: BoxDecoration(
                  color: Orbita.panel,
                  border: Border.all(color: Orbita.line),
                  borderRadius: BorderRadius.circular(18),
                ),
              ),
            ),
          ),
        );
    return Semantics(
      label: 'Cargando conversación',
      child: Column(
        children: <Widget>[
          bar(.62, 40, Alignment.centerRight, 0),
          const SizedBox(height: 12),
          bar(.88, 96, Alignment.centerLeft, .22),
          const SizedBox(height: 12),
          bar(.54, 56, Alignment.centerLeft, .44),
        ],
      ),
    );
  }
}
