import 'dart:convert';

import 'package:flutter/foundation.dart';

/// Bodies larger than this are decoded on a background isolate so a big
/// transcript never janks a frame. Below it, spawning an isolate costs more
/// (time and a transient heap) than decoding inline: AOT `jsonDecode` does
/// very roughly 50 KB per millisecond on a mid-range phone (estimate), and
/// the common bodies are 8–80 KB (transcripts, the notices long-poll).
const int kOffThreadJsonBytes = 384 * 1024;

Object? _decodeBytes(Uint8List bytes) => jsonDecode(utf8.decode(bytes, allowMalformed: true));

/// Decodes a JSON body. Throws [FormatException] when it is not JSON.
Future<Object?> decodeJsonBody(Uint8List bytes) async {
  if (bytes.isEmpty) return null;
  if (bytes.length >= kOffThreadJsonBytes) {
    return compute(_decodeBytes, bytes, debugLabel: 'json');
  }
  return _decodeBytes(bytes);
}
