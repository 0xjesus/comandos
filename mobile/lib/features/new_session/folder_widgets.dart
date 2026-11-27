import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'folder_search.dart';
import 'wizard_model.dart';

// The rows and fields of the «Carpeta» section (folder_search.dart holds the
// state). Touch targets are at least 44 px; colours are the Órbita tokens.

/// The search field: search icon, the text, a spinner while a request is on
/// its way and a clear button.
class FolderField extends StatelessWidget {
  const FolderField({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.loading,
    required this.onChanged,
    required this.onSubmitted,
    required this.onClear,
    this.showClear = false,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final bool loading;
  final ValueChanged<String> onChanged;
  final ValueChanged<String> onSubmitted;
  final VoidCallback onClear;

  /// The clear button also shows on an empty field (back to the chosen folder).
  final bool showClear;

  @override
  Widget build(BuildContext context) {
    final bool focused = focusNode.hasFocus;
    return Container(
      constraints: const BoxConstraints(minHeight: 48),
      decoration: BoxDecoration(
        color: Orbita.panel,
        border: Border.all(color: focused ? Orbita.brand : Orbita.line2),
        borderRadius: BorderRadius.circular(12),
      ),
      padding: const EdgeInsets.only(left: 12),
      child: Row(
        children: <Widget>[
          WebIcon(WebIcons.search, size: 17, color: focused ? Orbita.brand : Orbita.dim),
          const SizedBox(width: 10),
          Expanded(
            child: TextField(
              key: const ValueKey<String>('ns-cwd'),
              controller: controller,
              focusNode: focusNode,
              autocorrect: false,
              enableSuggestions: false,
              keyboardType: TextInputType.url,
              textInputAction: TextInputAction.search,
              onChanged: onChanged,
              onSubmitted: onSubmitted,
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 15, color: Orbita.text),
              cursorColor: Orbita.brand,
              decoration: const InputDecoration(
                isCollapsed: true,
                filled: false,
                border: InputBorder.none,
                enabledBorder: InputBorder.none,
                focusedBorder: InputBorder.none,
                contentPadding: EdgeInsets.symmetric(vertical: 13),
                hintText: 'Busca un proyecto o escribe una ruta',
                hintStyle: TextStyle(fontFamily: Orbita.sans, fontSize: 15, color: Orbita.faint),
              ),
            ),
          ),
          if (loading)
            const Padding(
              padding: EdgeInsets.symmetric(horizontal: 6),
              child: SizedBox.square(
                key: ValueKey<String>('ns-cwd-loading'),
                dimension: 14,
                child: CircularProgressIndicator(strokeWidth: 1.8, color: Orbita.dim),
              ),
            ),
          if (controller.text.isNotEmpty || showClear)
            Semantics(
              button: true,
              label: 'Borrar',
              child: GestureDetector(
                key: const ValueKey<String>('ns-cwd-clear'),
                behavior: HitTestBehavior.opaque,
                onTap: onClear,
                child: const SizedBox(
                  width: 44,
                  height: 46,
                  child: Center(child: WebIcon(WebIcons.close, size: 16, color: Orbita.dim)),
                ),
              ),
            )
          else
            const SizedBox(width: 12),
        ],
      ),
    );
  }
}

/// A small pill: «reciente», a model tag.
class NsBadge extends StatelessWidget {
  const NsBadge(this.text, {super.key, this.color = Orbita.brand});

  final String text;
  final Color color;

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
        decoration: BoxDecoration(
          color: webMix(color, .14, Orbita.panel),
          borderRadius: BorderRadius.circular(99),
          border: Border.all(color: webMix(color, .35, Orbita.line)),
        ),
        child: Text(
          text,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontFamily: Orbita.mono, fontSize: 10, fontWeight: FontWeight.w600, color: color, height: 1.3),
        ),
      );
}

/// A tappable row with the Órbita press tint.
class _RowTap extends StatelessWidget {
  const _RowTap({required this.child, required this.onTap, this.rowKey});

  final Widget child;
  final VoidCallback? onTap;
  final Key? rowKey;

  @override
  Widget build(BuildContext context) => Material(
        type: MaterialType.transparency,
        child: InkWell(
          key: rowKey,
          onTap: onTap,
          borderRadius: BorderRadius.circular(10),
          splashColor: webMix(Orbita.brand, .12, Colors.transparent),
          highlightColor: webMix(Orbita.brand, .08, Colors.transparent),
          child: child,
        ),
      );
}

/// One folder: icon (branch for a git repo), the name bold with the matched
/// letters highlighted, `~/…` dimmed under it, «reciente» for live sessions.
class FolderRow extends StatelessWidget {
  const FolderRow({
    super.key,
    required this.entry,
    required this.onTap,
    this.query = '',
    this.showRecent = true,
    this.compact = false,
  });

  final FolderEntry entry;
  final VoidCallback onTap;
  final String query;
  final bool showRecent;

  /// A subfolder of the chosen folder: the name only, 46 px.
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final List<(String, bool)> runs = WizardPaths.highlight(entry.name, query);
    return _RowTap(
      rowKey: ValueKey<String>('ns-folder:${entry.path}'),
      onTap: onTap,
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: compact ? 46 : 54),
        child: Padding(
          padding: EdgeInsets.symmetric(horizontal: 10, vertical: compact ? 4 : 7),
          child: Row(
            children: <Widget>[
              Container(
                width: compact ? 28 : 34,
                height: compact ? 28 : 34,
                decoration: BoxDecoration(
                  color: Orbita.panel2,
                  borderRadius: BorderRadius.circular(9),
                  border: Border.all(color: Orbita.line),
                ),
                child: Center(
                  child: WebIcon(
                    entry.repo ? WebIcons.gitBranch : WebIcons.folder,
                    size: 16,
                    color: entry.repo ? Orbita.brand : Orbita.dim,
                  ),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    Row(
                      children: <Widget>[
                        Flexible(
                          child: Text.rich(
                            TextSpan(
                              children: <InlineSpan>[
                                for (final (String text, bool hit) in runs)
                                  TextSpan(
                                    text: text,
                                    style: hit
                                        ? TextStyle(
                                            color: Orbita.brand,
                                            backgroundColor: webMix(Orbita.brand, .16, Colors.transparent),
                                          )
                                        : null,
                                  ),
                              ],
                            ),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(
                              fontFamily: Orbita.sans,
                              fontSize: 15,
                              fontWeight: FontWeight.w700,
                              color: Orbita.text,
                            ),
                          ),
                        ),
                        if (showRecent && entry.recent) ...<Widget>[
                          const SizedBox(width: 8),
                          const NsBadge('reciente', color: Orbita.done),
                        ],
                      ],
                    ),
                    if (!compact) ...<Widget>[
                      const SizedBox(height: 2),
                      Text(
                        entry.short,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11.5, color: Orbita.faint),
                      ),
                    ],
                  ],
                ),
              ),
              const SizedBox(width: 6),
              const WebIcon(WebIcons.chevronRight, size: 14, color: Orbita.faint),
            ],
          ),
        ),
      ),
    );
  }
}

/// The chosen folder: check, name and short path, «Cambiar».
class FolderPill extends StatelessWidget {
  const FolderPill({super.key, required this.name, required this.short, required this.missing, required this.onChange});

  final String name;
  final String short;
  final bool missing;
  final VoidCallback onChange;

  @override
  Widget build(BuildContext context) {
    final Color accent = missing ? Orbita.waiting : Orbita.done;
    return Container(
      key: const ValueKey<String>('ns-picked'),
      decoration: BoxDecoration(
        color: webMix(accent, .08, Orbita.panel),
        border: Border.all(color: webMix(accent, .45, Orbita.line)),
        borderRadius: BorderRadius.circular(12),
      ),
      padding: const EdgeInsets.fromLTRB(12, 8, 6, 8),
      child: Row(
        children: <Widget>[
          Container(
            width: 30,
            height: 30,
            decoration: BoxDecoration(color: webMix(accent, .18, Orbita.panel), shape: BoxShape.circle),
            child: Center(child: WebIcon(missing ? WebIcons.plus : WebIcons.check, size: 15, color: accent)),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontFamily: Orbita.sans,
                    fontSize: 15,
                    fontWeight: FontWeight.w700,
                    color: Orbita.text,
                  ),
                ),
                const SizedBox(height: 2),
                Text(
                  short,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11.5, color: Orbita.dim),
                ),
              ],
            ),
          ),
          const SizedBox(width: 6),
          TextButton(
            key: const ValueKey<String>('ns-change'),
            onPressed: onChange,
            style: TextButton.styleFrom(
              minimumSize: const Size(44, 44),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              foregroundColor: Orbita.brand,
              textStyle: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600),
            ),
            child: const Text('Cambiar'),
          ),
        ],
      ),
    );
  }
}

/// Breadcrumbs of a path, scrolled to the end; each segment is a 44 px target.
class NsCrumbs extends StatelessWidget {
  const NsCrumbs({super.key, required this.base, required this.home, required this.onTap});

  final String base;
  final String? home;
  final ValueChanged<String> onTap;

  @override
  Widget build(BuildContext context) {
    final List<(String, String)> crumbs = WizardPaths.crumbs(base);
    // `~` stands for the HOME segments.
    int skip = 0;
    if (home != null && home!.isNotEmpty && base.startsWith(home!)) {
      skip = WizardPaths.crumbs(home!).length;
    }
    final List<(String, String)> shown = <(String, String)>[
      if (skip > 0) ('~', home!),
      ...crumbs.skip(skip),
    ];
    return SingleChildScrollView(
      key: const ValueKey<String>('ns-crumbs'),
      scrollDirection: Axis.horizontal,
      reverse: true,
      child: Row(
        children: <Widget>[
          for (int i = 0; i < shown.length; i++) ...<Widget>[
            if (i > 0) const Text('/', style: TextStyle(fontFamily: Orbita.mono, fontSize: 12, color: Orbita.line2)),
            _RowTap(
              onTap: () => onTap(shown[i].$2),
              child: ConstrainedBox(
                constraints: const BoxConstraints(minHeight: 44, minWidth: 32),
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 7),
                  child: Center(
                    widthFactor: 1,
                    child: Text(
                      shown[i].$1,
                      style: TextStyle(
                        fontFamily: Orbita.mono,
                        fontSize: 12.5,
                        fontWeight: i == shown.length - 1 ? FontWeight.w700 : FontWeight.w400,
                        color: i == shown.length - 1 ? Orbita.text : Orbita.dim,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// «~/x no existe» with «Crear carpeta».
class MissingFolder extends StatelessWidget {
  const MissingFolder({super.key, required this.short, required this.onCreate});

  final String short;
  final VoidCallback onCreate;

  @override
  Widget build(BuildContext context) => Container(
        decoration: BoxDecoration(
          color: webMix(Orbita.waiting, .07, Orbita.panel),
          border: Border.all(color: webMix(Orbita.waiting, .35, Orbita.line)),
          borderRadius: BorderRadius.circular(12),
        ),
        padding: const EdgeInsets.fromLTRB(12, 6, 6, 6),
        child: Row(
          children: <Widget>[
            const WebIcon(WebIcons.sparkles, size: 15, color: Orbita.waiting),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                '$short no existe',
                style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.waiting),
              ),
            ),
            TextButton.icon(
              key: const ValueKey<String>('ns-mkdir'),
              onPressed: onCreate,
              icon: const WebIcon(WebIcons.plus, size: 14, color: Orbita.text),
              label: const Text('Crear carpeta'),
              style: TextButton.styleFrom(
                minimumSize: const Size(44, 44),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                foregroundColor: Orbita.text,
                backgroundColor: Orbita.panel2,
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(10),
                  side: const BorderSide(color: Orbita.line2),
                ),
                textStyle: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, fontWeight: FontWeight.w600),
              ),
            ),
          ],
        ),
      );
}

/// A dimmed one-line hint under the list («Sin resultados…»).
class NsHint extends StatelessWidget {
  const NsHint(this.text, {super.key, this.color = Orbita.faint});

  final String text;
  final Color color;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 10),
        child: Text(text, style: TextStyle(fontFamily: Orbita.sans, fontSize: 13, height: 1.4, color: color)),
      );
}
