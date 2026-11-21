import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Item 12: nothing of the app leaves the phone through cloud backup or a
/// device-to-device transfer (outbox and drafts hold message text).
void main() {
  const String res = 'android/app/src/main/res/xml';
  final String manifest = File('android/app/src/main/AndroidManifest.xml').readAsStringSync();

  test('the manifest points at backup rules that exclude everything', () {
    expect(manifest, contains('android:allowBackup="false"'));
    expect(manifest, contains('android:dataExtractionRules="@xml/data_extraction_rules"'));
    expect(manifest, contains('android:fullBackupContent="@xml/backup_rules"'));
  });

  List<String> excluded(String xml, String section) {
    final RegExpMatch? m = RegExp('<$section>(.*?)</$section>', dotAll: true).firstMatch(xml);
    expect(m, isNotNull, reason: section);
    final String body = m!.group(1)!;
    expect(body, isNot(contains('<include')));
    return <String>[
      for (final RegExpMatch e in RegExp(r'<exclude domain="([a-z_]+)" path="\."\s*/>').allMatches(body)) e.group(1)!,
    ];
  }

  test('Android 12+: cloud backup and device transfer exclude every domain', () {
    final String xml = File('$res/data_extraction_rules.xml').readAsStringSync();
    const Set<String> all = <String>{
      'root', 'file', 'database', 'sharedpref', 'external',
      'device_root', 'device_file', 'device_database', 'device_sharedpref',
    };
    expect(excluded(xml, 'cloud-backup').toSet(), all);
    expect(excluded(xml, 'device-transfer').toSet(), all);
  });

  test('Android 11 and older: full backup excludes every domain', () {
    final String xml = File('$res/backup_rules.xml').readAsStringSync();
    expect(excluded(xml, 'full-backup-content').toSet(),
        <String>{'root', 'file', 'database', 'sharedpref', 'external'});
  });
}
