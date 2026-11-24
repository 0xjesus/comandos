import 'package:flutter/painting.dart';

import '../../ui/theme.dart';

/// What a piece of highlighted code is (`comandos_core::chat_markdown::Tok`).
enum Tok { plain, keyword, string, number, comment, function, kind, added, removed, meta }

enum _Lang { rust, js, python, shell, json, conf, go, clike, sql, css, markup, diff, plain }

_Lang _langOf(String name, String body) => switch (name) {
      'rust' || 'rs' => _Lang.rust,
      'js' || 'javascript' || 'jsx' || 'ts' || 'typescript' || 'tsx' || 'mjs' || 'cjs' => _Lang.js,
      'py' || 'python' || 'python3' => _Lang.python,
      'sh' || 'bash' || 'zsh' || 'shell' || 'console' || 'shellsession' || 'fish' => _Lang.shell,
      'json' || 'jsonc' || 'json5' || 'jsonl' => _Lang.json,
      'toml' || 'yaml' || 'yml' || 'ini' || 'env' || 'dotenv' || 'conf' || 'nginx' || 'dockerfile' || 'make' || 'makefile' =>
        _Lang.conf,
      'go' || 'golang' => _Lang.go,
      'c' || 'h' || 'cpp' || 'c++' || 'hpp' || 'java' || 'kotlin' || 'kt' || 'swift' || 'cs' || 'csharp' || 'dart' || 'php' ||
      'scala' =>
        _Lang.clike,
      'sql' || 'psql' || 'sqlite' || 'postgres' || 'postgresql' || 'mysql' => _Lang.sql,
      'css' || 'scss' || 'less' => _Lang.css,
      'html' || 'xml' || 'svg' || 'vue' || 'svelte' => _Lang.markup,
      'diff' || 'patch' => _Lang.diff,
      '' when _looksLikeDiff(body) => _Lang.diff,
      _ => _Lang.plain,
    };

bool _looksLikeDiff(String body) {
  final List<String> lines = body.split('\n').where((String l) => l.isNotEmpty).toList();
  return lines.any((String l) => l.startsWith('@@') || l.startsWith('+++ ') || l.startsWith('--- ')) &&
      lines.every((String l) =>
          l.startsWith('+') || l.startsWith('-') || l.startsWith(' ') || l.startsWith('@') || l.startsWith('diff ') || l.startsWith('index '));
}

const Map<_Lang, Set<String>> _keywords = <_Lang, Set<String>>{
  _Lang.rust: <String>{
    'as', 'async', 'await', 'break', 'const', 'continue', 'crate', 'dyn', 'else', 'enum', 'extern', 'false', 'fn', 'for', 'if', //
    'impl', 'in', 'let', 'loop', 'match', 'mod', 'move', 'mut', 'pub', 'ref', 'return', 'self', 'Self', 'static', 'struct',
    'super', 'trait', 'true', 'type', 'unsafe', 'use', 'where', 'while', 'Some', 'None', 'Ok', 'Err',
  },
  _Lang.js: <String>{
    'async', 'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'default', 'delete', 'do', 'else', 'export', //
    'extends', 'false', 'finally', 'for', 'from', 'function', 'if', 'import', 'in', 'instanceof', 'let', 'new', 'null', 'of',
    'return', 'static', 'super', 'switch', 'this', 'throw', 'true', 'try', 'typeof', 'undefined', 'var', 'void', 'while',
    'yield', 'interface', 'type', 'enum', 'implements', 'readonly', 'as', 'declare', 'namespace', 'private', 'public',
    'protected',
  },
  _Lang.python: <String>{
    'and', 'as', 'assert', 'async', 'await', 'break', 'class', 'continue', 'def', 'del', 'elif', 'else', 'except', 'False', //
    'finally', 'for', 'from', 'global', 'if', 'import', 'in', 'is', 'lambda', 'None', 'nonlocal', 'not', 'or', 'pass',
    'raise', 'return', 'True', 'try', 'while', 'with', 'yield', 'self', 'match', 'case',
  },
  _Lang.shell: <String>{
    'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'until', 'do', 'done', 'case', 'esac', 'in', 'function', 'return', //
    'export', 'local', 'readonly', 'set', 'unset', 'exit', 'source', 'trap', 'shift',
  },
  _Lang.json: <String>{'true', 'false', 'null'},
  _Lang.conf: <String>{'true', 'false', 'null', 'yes', 'no', 'on', 'off'},
  _Lang.go: <String>{
    'break', 'case', 'chan', 'const', 'continue', 'default', 'defer', 'else', 'fallthrough', 'for', 'func', 'go', 'goto', 'if', //
    'import', 'interface', 'map', 'package', 'range', 'return', 'select', 'struct', 'switch', 'type', 'var', 'nil', 'true',
    'false',
  },
  _Lang.clike: <String>{
    'auto', 'break', 'case', 'catch', 'class', 'const', 'continue', 'default', 'delete', 'do', 'else', 'enum', 'extends', //
    'extern', 'false', 'final', 'for', 'fun', 'func', 'if', 'implements', 'import', 'include', 'interface', 'let', 'namespace',
    'new', 'null', 'nullptr', 'override', 'package', 'private', 'protected', 'public', 'return', 'static', 'struct', 'switch',
    'template', 'this', 'throw', 'true', 'try', 'typedef', 'using', 'val', 'var', 'virtual', 'void', 'while', 'self',
  },
  _Lang.sql: <String>{
    'select', 'from', 'where', 'insert', 'into', 'values', 'update', 'set', 'delete', 'join', 'left', 'right', 'inner', 'outer', //
    'on', 'group', 'by', 'order', 'limit', 'offset', 'having', 'create', 'table', 'index', 'view', 'drop', 'alter', 'add',
    'primary', 'key', 'foreign', 'references', 'not', 'null', 'and', 'or', 'as', 'distinct', 'union', 'all', 'case', 'when',
    'then', 'else', 'end', 'in', 'is', 'like', 'exists', 'default', 'begin', 'commit', 'with', 'returning', 'asc', 'desc',
  },
};

final RegExp _letter = RegExp(r'^\p{L}$', unicode: true);
final RegExp _alnum = RegExp(r'^[\p{L}\p{N}]$', unicode: true);
final RegExp _upper = RegExp(r'^\p{Lu}$', unicode: true);
final RegExp _space = RegExp(r'^\s$', unicode: true);

bool _isDigit(int c) => c >= 0x30 && c <= 0x39;
bool _isAsciiAlnum(int c) => _isDigit(c) || (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A);
bool _isAlpha(int c) => (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) || (c > 127 && _letter.hasMatch(String.fromCharCode(c)));
bool _isAlnum(int c) => _isAsciiAlnum(c) || (c > 127 && _alnum.hasMatch(String.fromCharCode(c)));
bool _isSpace(int c) => c == 32 || c == 9 || c == 10 || c == 13 || (c > 127 && _space.hasMatch(String.fromCharCode(c)));

/// Fenced code split into highlighted pieces, as the web and the desktop split
/// it (`comandos_core::chat_markdown::tokens`); concatenated, they are exactly
/// [body]. Unknown languages come back as one plain piece.
List<(Tok, String)> codeTokens(String langName, String body) {
  final _Lang lang = _langOf(langName.toLowerCase(), body);
  final List<(Tok, String)> out = <(Tok, String)>[];
  void push(Tok kind, String text) {
    if (text.isEmpty) return;
    if (out.isNotEmpty && out.last.$1 == kind) {
      out[out.length - 1] = (kind, out.last.$2 + text);
    } else {
      out.add((kind, text));
    }
  }

  if (lang == _Lang.plain) {
    push(Tok.plain, body);
    return out;
  }
  if (lang == _Lang.diff) {
    for (final (int i, String line) in body.split('\n').indexed) {
      if (i > 0) push(Tok.plain, '\n');
      final Tok kind = line.startsWith('+++') || line.startsWith('---') || line.startsWith('@@') || line.startsWith('diff ')
          ? Tok.meta
          : line.startsWith('+')
              ? Tok.added
              : line.startsWith('-')
                  ? Tok.removed
                  : Tok.plain;
      push(kind, line);
    }
    return out;
  }
  final Set<String> words = _keywords[lang] ?? const <String>{};
  final List<int> chars = body.runes.toList();
  final int n = chars.length;
  String text(int from, int to) => String.fromCharCodes(chars, from, to > n ? n : to);
  bool starts(int i, String s) {
    final List<int> r = s.runes.toList();
    if (i + r.length > n) return false;
    for (int k = 0; k < r.length; k++) {
      if (chars[i + k] != r[k]) return false;
    }
    return true;
  }

  int find(int from, int c) {
    for (int k = from; k < n; k++) {
      if (chars[k] == c) return k;
    }
    return -1;
  }

  final List<String> lineComment = switch (lang) {
    _Lang.python || _Lang.shell || _Lang.conf => const <String>['#'],
    _Lang.sql => const <String>['--'],
    _Lang.json || _Lang.css || _Lang.markup => const <String>[],
    _ => const <String>['//'],
  };
  final bool blockComment = lang == _Lang.rust || lang == _Lang.js || lang == _Lang.go || lang == _Lang.clike || lang == _Lang.css || lang == _Lang.sql;
  bool lineStart = true;
  int i = 0;
  while (i < n) {
    final int c = chars[i];
    // Comments.
    String? mark;
    for (final String m in lineComment) {
      if (starts(i, m)) {
        mark = m;
        break;
      }
    }
    if (mark != null) {
      final bool wordStart = i == 0 || _isSpace(chars[i - 1]);
      if (mark != '#' || wordStart) {
        final int nl = find(i, 10);
        final int end = nl < 0 ? n : nl;
        push(Tok.comment, text(i, end));
        i = end;
        continue;
      }
    }
    if (blockComment && starts(i, '/*')) {
      int end = n;
      for (int k = i + 2; k < n; k++) {
        if (starts(k, '*/')) {
          end = k + 2;
          break;
        }
      }
      push(Tok.comment, text(i, end));
      i = end;
      continue;
    }
    if (lang == _Lang.markup && starts(i, '<!--')) {
      int end = n;
      for (int k = i + 4; k < n; k++) {
        if (starts(k, '-->')) {
          end = k + 3;
          break;
        }
      }
      push(Tok.comment, text(i, end));
      i = end;
      continue;
    }
    // Strings.
    final bool quote = switch (c) {
      0x22 => true, // "
      0x27 => lang != _Lang.rust || (i + 2 < n && chars[i + 2] == 0x27) || (i + 1 < n && chars[i + 1] == 0x5C), // '
      0x60 => lang == _Lang.js || lang == _Lang.shell || lang == _Lang.go, // `
      _ => false,
    };
    if (quote && lang != _Lang.markup) {
      final String q = String.fromCharCode(c);
      final bool triple = lang == _Lang.python && starts(i, q * 3);
      int k = i + (triple ? 3 : 1);
      while (k < n) {
        final int d = chars[k];
        if (d == 0x5C) {
          k += 2;
          continue;
        }
        if (triple) {
          if (starts(k, q * 3)) {
            k += 3;
            break;
          }
        } else if (d == c) {
          k += 1;
          break;
        } else if (d == 10 && c != 0x60) {
          break;
        }
        k += 1;
      }
      final int end = k < n ? k : n;
      // A JSON key is the string right before a colon.
      bool isKey = false;
      if (lang == _Lang.json) {
        for (int j = end; j < n; j++) {
          if (!_isSpace(chars[j])) {
            isKey = chars[j] == 0x3A;
            break;
          }
        }
      }
      push(isKey ? Tok.function : Tok.string, text(i, end));
      i = end;
      lineStart = false;
      continue;
    }
    // Markup tags.
    if (lang == _Lang.markup && c == 0x3C) {
      final int gt = find(i, 0x3E);
      final int end = gt < 0 ? n : gt + 1;
      push(Tok.keyword, text(i, end));
      i = end;
      continue;
    }
    // Numbers.
    if (_isDigit(c) && !(i > 0 && (_isAlnum(chars[i - 1]) || chars[i - 1] == 0x5F))) {
      int end = i;
      while (end < n && (_isAsciiAlnum(chars[end]) || chars[end] == 0x2E || chars[end] == 0x5F)) {
        end++;
      }
      push(Tok.number, text(i, end));
      i = end;
      lineStart = false;
      continue;
    }
    // Shell variables.
    if (lang == _Lang.shell && c == 0x24) {
      int end;
      if (i + 1 < n && chars[i + 1] == 0x7B) {
        final int close = find(i, 0x7D);
        end = close < 0 ? n : close + 1;
      } else {
        end = i + 1;
        while (end < n && (_isAlnum(chars[end]) || chars[end] == 0x5F)) {
          end++;
        }
      }
      push(Tok.kind, text(i, end));
      i = end;
      continue;
    }
    // Words.
    if (_isAlpha(c) || c == 0x5F || (lang == _Lang.css && c == 0x2D)) {
      final bool dash = lang == _Lang.css || lang == _Lang.shell;
      int end = i;
      while (end < n && (_isAlnum(chars[end]) || chars[end] == 0x5F || (dash && chars[end] == 0x2D))) {
        end++;
      }
      final String word = text(i, end);
      int? next;
      for (int j = end; j < n; j++) {
        if (chars[j] != 0x20) {
          next = chars[j];
          break;
        }
      }
      final bool isKw = lang == _Lang.sql ? words.contains(word.toLowerCase()) : words.contains(word);
      // Calls, shell commands, CSS properties and config keys.
      final bool isName = (lang == _Lang.shell && lineStart) ||
          ((next == 0x28 || (lang == _Lang.rust && next == 0x21)) && lang != _Lang.css) ||
          (lang == _Lang.css && next == 0x3A) ||
          (lang == _Lang.conf && lineStart && (next == 0x3D || next == 0x3A));
      final Tok kind = isKw
          ? Tok.keyword
          : isName
              ? Tok.function
              : (_upper.hasMatch(String.fromCharCode(word.runes.first)) &&
                      (lang == _Lang.rust || lang == _Lang.js || lang == _Lang.go || lang == _Lang.clike))
                  ? Tok.kind
                  : Tok.plain;
      push(kind, word);
      i = end;
      lineStart = false;
      continue;
    }
    if (c == 10) {
      lineStart = true;
    } else if (!_isSpace(c) && (lang != _Lang.shell || !(c == 0x7C || c == 0x3B || c == 0x26 || c == 0x28))) {
      lineStart = false;
    }
    push(Tok.plain, String.fromCharCode(c));
    i += 1;
  }
  return out;
}

/// The colours of `#chat-view .tk-*` (index_page.css) in the app's palette.
TextStyle? tokStyle(Tok t) => switch (t) {
      Tok.plain => null,
      Tok.keyword => const TextStyle(color: Orbita.brand, fontWeight: FontWeight.w600),
      Tok.string => const TextStyle(color: Orbita.done),
      Tok.number => const TextStyle(color: Orbita.waiting),
      Tok.comment => const TextStyle(color: Orbita.faint, fontStyle: FontStyle.italic),
      Tok.function => const TextStyle(color: Orbita.working),
      Tok.kind => const TextStyle(color: Color(0xFF58C2D3)), // working 55% + ok
      Tok.added => TextStyle(color: Orbita.done, backgroundColor: Orbita.done.withValues(alpha: .12)),
      Tok.removed => TextStyle(color: Orbita.err, backgroundColor: Orbita.err.withValues(alpha: .12)),
      Tok.meta => const TextStyle(color: Orbita.working),
    };

/// Code past this size is drawn plain: colouring it would cost more than it helps.
const int kHighlightLimit = 60000;

/// Highlighted spans for [code] (plain past [kHighlightLimit]).
List<TextSpan> codeSpans(String lang, String code) {
  if (code.length > kHighlightLimit) return <TextSpan>[TextSpan(text: code)];
  return <TextSpan>[for (final (Tok t, String s) in codeTokens(lang, code)) TextSpan(text: s, style: tokStyle(t))];
}
