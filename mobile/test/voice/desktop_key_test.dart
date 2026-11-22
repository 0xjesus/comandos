import 'package:comandos_mobile/features/voice/openai_key_store.dart';
import 'package:flutter_test/flutter_test.dart';

/// A fake key built at run time: no real key in the repository.
final String fakeKey = 'sk-${'prueba' * 4}';

void main() {
  test('with no key on the phone, takes the desktop key and keeps it', () async {
    final MemorySecretStore secrets = MemorySecretStore();
    int asked = 0;
    final OpenAiKeyStore keys = OpenAiKeyStore(secrets, () async {
      asked++;
      return fakeKey;
    });
    await keys.load();
    expect(await keys.read(), fakeKey);
    expect(keys.status.value.present, isTrue);
    expect(secrets.values.containsValue(fakeKey), isTrue);
    // Kept: a new app start reads it from the phone and does not ask again.
    final OpenAiKeyStore again = OpenAiKeyStore(secrets, () async {
      asked++;
      return null;
    });
    await again.load();
    expect(await again.read(), fakeKey);
    expect(asked, 1);
  });

  test('a key on the phone wins and the desktop is not asked', () async {
    final MemorySecretStore secrets = MemorySecretStore();
    final String mine = 'sk-${'propia' * 4}';
    await OpenAiKeyStore(secrets).save(mine);
    bool asked = false;
    final OpenAiKeyStore keys = OpenAiKeyStore(secrets, () async {
      asked = true;
      return fakeKey;
    });
    expect(await keys.read(), mine);
    expect(asked, isFalse);
  });

  test('clearing the key stops taking the desktop one until a key is saved', () async {
    final MemorySecretStore secrets = MemorySecretStore();
    final OpenAiKeyStore keys = OpenAiKeyStore(secrets, () async => fakeKey);
    await keys.load();
    await keys.clear();
    final OpenAiKeyStore after = OpenAiKeyStore(secrets, () async => fakeKey);
    await after.load();
    expect(await after.read(), isNull);
    expect(after.status.value.present, isFalse);
    await after.save(fakeKey);
    final OpenAiKeyStore later = OpenAiKeyStore(secrets, () async => null);
    expect(await later.read(), fakeKey);
  });

  test('no answer from the desktop: no key, and it asks again only after a minute', () async {
    final MemorySecretStore secrets = MemorySecretStore();
    int asked = 0;
    final OpenAiKeyStore keys = OpenAiKeyStore(secrets, () async {
      asked++;
      throw Exception('sin red');
    });
    await keys.load();
    await keys.load();
    expect(await keys.read(), isNull);
    expect(keys.status.value.loaded, isTrue);
    expect(asked, 1);
  });

  test('something that is not one clear key is ignored', () async {
    final OpenAiKeyStore keys = OpenAiKeyStore(MemorySecretStore(), () async => 'nada');
    expect(await keys.read(), isNull);
  });
}
