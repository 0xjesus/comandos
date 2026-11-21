import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/locator.dart';
import '../../core/models/transcript.dart';
import '../../ui/theme.dart';
import '../voice/voice.dart';
import 'markdown.dart';

/// Rendered text per message is capped; "ver más" shows the rest.
const int kCollapsedChars = 2500;

class _Rendered {
  _Rendered(this.collapsed, this.isCut);
  final List<MdBlock> collapsed;
  final bool isCut;
  List<MdBlock>? full;
}

String _cut(String text) {
  if (text.length <= kCollapsedChars) return text;
  final int nl = text.lastIndexOf('\n', kCollapsedChars);
  final int at = nl > kCollapsedChars ~/ 2 ? nl : kCollapsedChars;
  String head = text.substring(0, at);
  // Do not leave a code fence open.
  if ('```'.allMatches(head).length.isOdd) head = '$head\n```';
  return head;
}

_Rendered _render(ChatMessage m) {
  final Object? cache = m.renderCache;
  if (cache is _Rendered) return cache;
  final bool cut = m.text.length > kCollapsedChars;
  final _Rendered r = _Rendered(parseMarkdown(cut ? _cut(m.text) : m.text), cut);
  m.renderCache = r;
  return r;
}

/// One transcript message (user bubble, assistant text, or tool lines).
class MessageView extends StatelessWidget {
  const MessageView(this.message, {super.key});

  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    switch (message.role) {
      case ChatRole.user:
        return _UserBubble(message);
      case ChatRole.assistant:
        return _AssistantText(message);
      case ChatRole.tool:
        return _ToolLines(message);
    }
  }
}

void _showMessageActions(BuildContext context, ChatMessage m) {
  final Speaker? speaker = sl.maybe<Speaker>();
  showModalBottomSheet<void>(
    context: context,
    backgroundColor: Orbita.panel,
    builder: (BuildContext c) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          ListTile(
            leading: const Icon(Icons.copy),
            title: const Text('Copiar texto'),
            onTap: () {
              Clipboard.setData(ClipboardData(text: m.text));
              Navigator.of(c).pop();
            },
          ),
          if (speaker != null && m.role == ChatRole.assistant)
            ListTile(
              leading: const Icon(Icons.volume_up_outlined),
              title: const Text('Leer en voz alta'),
              onTap: () {
                Navigator.of(c).pop();
                speaker.speak(m.text);
              },
            ),
        ],
      ),
    ),
  );
}

class _UserBubble extends StatelessWidget {
  const _UserBubble(this.message);

  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.centerRight,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(48, 6, 12, 6),
        child: GestureDetector(
          onLongPress: () => _showMessageActions(context, message),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: Orbita.panel2,
              borderRadius: BorderRadius.circular(16),
              border: Border.all(color: Orbita.line),
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
              child: _Expandable(message: message, color: Orbita.text),
            ),
          ),
        ),
      ),
    );
  }
}

class _AssistantText extends StatelessWidget {
  const _AssistantText(this.message);

  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(14, 8, 14, 8),
      child: GestureDetector(
        onLongPress: () => _showMessageActions(context, message),
        child: _Expandable(message: message, color: Orbita.text),
      ),
    );
  }
}

/// Markdown with a "ver más" for long messages. Stateful only to remember
/// the expansion.
class _Expandable extends StatefulWidget {
  const _Expandable({required this.message, required this.color});

  final ChatMessage message;
  final Color color;

  @override
  State<_Expandable> createState() => _ExpandableState();
}

class _ExpandableState extends State<_Expandable> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final _Rendered r = _render(widget.message);
    if (!r.isCut) return MarkdownView(r.collapsed, color: widget.color);
    final List<MdBlock> blocks =
        _open ? (r.full ??= parseMarkdown(widget.message.text)) : r.collapsed;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        MarkdownView(blocks, color: widget.color),
        TextButton(
          onPressed: () => setState(() => _open = !_open),
          child: Text(_open ? 'ver menos' : 'ver más'),
        ),
      ],
    );
  }
}

/// Tool calls: collapsed to the first line in a subdued style.
class _ToolLines extends StatefulWidget {
  const _ToolLines(this.message);

  final ChatMessage message;

  @override
  State<_ToolLines> createState() => _ToolLinesState();
}

class _ToolLinesState extends State<_ToolLines> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final List<String> lines = widget.message.text.split('\n');
    final String shown = _open ? widget.message.text : lines.first;
    final int more = lines.length - 1;
    return InkWell(
      onTap: more > 0 ? () => setState(() => _open = !_open) : null,
      onLongPress: () => _showMessageActions(context, widget.message),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 36),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(14, 6, 14, 6),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              const Padding(
                padding: EdgeInsets.only(top: 2),
                child: Icon(Icons.build_outlined, size: 14, color: Orbita.faint),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  shown,
                  maxLines: _open ? null : 1,
                  overflow: _open ? null : TextOverflow.ellipsis,
                  style: const TextStyle(
                      fontFamily: Orbita.mono, fontSize: 12.5, color: Orbita.faint, height: 1.4),
                ),
              ),
              if (more > 0 && !_open)
                Text('  +$more', style: const TextStyle(color: Orbita.faint, fontSize: 12)),
            ],
          ),
        ),
      ),
    );
  }
}
