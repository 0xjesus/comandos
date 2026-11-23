import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../ui/theme.dart';
import '../pairing/qr_scan_screen.dart';
import 'openai_client.dart';
import 'openai_key_store.dart';
import 'speech.dart';
import 'voice_services.dart';
import 'voice_settings.dart';

/// Ajustes › Voz, as a standalone screen (opened when the key is missing).
class VoiceSettingsScreen extends StatelessWidget {
  const VoiceSettingsScreen({super.key});

  static const String routeName = '/settings/voice';

  @override
  Widget build(BuildContext context) => Scaffold(
        appBar: AppBar(title: const Text('Voz')),
        body: SafeArea(top: false, child: ListView(children: const <Widget>[VoiceSettingsSection()])),
      );
}

/// The "Voz" block embedded in the settings screen: OpenAI key and voice
/// preferences.
class VoiceSettingsSection extends StatefulWidget {
  const VoiceSettingsSection({super.key});

  @override
  State<VoiceSettingsSection> createState() => _VoiceSettingsSectionState();
}

class _VoiceSettingsSectionState extends State<VoiceSettingsSection> {
  final VoiceServices _v = VoiceServices.instance;
  String? _testResult;
  bool _testOk = false;
  bool _testing = false;

  @override
  void initState() {
    super.initState();
    unawaited(_v.ready());
  }

  void _say(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _store(String raw, {bool fromClipboard = false}) async {
    final String? key = extractOpenAiKey(raw);
    if (key == null) {
      _say('Eso no parece una llave de OpenAI (empieza con «sk-»).');
      return;
    }
    try {
      await _v.keys.save(key);
    } catch (_) {
      _say('No pude guardar la llave en el almacén seguro.');
      return;
    }
    if (fromClipboard) await Clipboard.setData(const ClipboardData(text: ''));
    setState(() => _testResult = null);
    _say(fromClipboard ? 'Llave guardada. Borré el portapapeles.' : 'Llave guardada.');
  }

  Future<void> _scan() async {
    final String? v = await Navigator.of(context).push<String>(MaterialPageRoute<String>(
      builder: (_) => const QrScanScreen(
        title: 'Escanea el código de tu llave',
        fallback: 'No pude abrir la cámara. Vuelve y pega la llave.',
      ),
    ));
    if (v != null) await _store(v);
  }

  Future<void> _paste() async {
    String? clip;
    try {
      clip = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
    } catch (_) {}
    if (clip != null && extractOpenAiKey(clip) != null) {
      await _store(clip, fromClipboard: true);
      return;
    }
    if (!mounted) return;
    final TextEditingController t = TextEditingController();
    final String? typed = await showDialog<String>(
      context: context,
      builder: (BuildContext c) => AlertDialog(
        backgroundColor: Orbita.panel,
        title: const Text('Pegar llave de OpenAI'),
        content: TextField(
          controller: t,
          obscureText: true,
          autocorrect: false,
          enableSuggestions: false,
          decoration: const InputDecoration(hintText: 'sk-…'),
        ),
        actions: <Widget>[
          TextButton(onPressed: () => Navigator.of(c).pop(), child: const Text('Cancelar')),
          TextButton(onPressed: () => Navigator.of(c).pop(t.text), child: const Text('Guardar')),
        ],
      ),
    );
    t.dispose();
    if (typed != null && typed.trim().isNotEmpty) await _store(typed);
  }

  Future<void> _test() async {
    setState(() {
      _testing = true;
      _testResult = null;
    });
    String result;
    bool ok = false;
    try {
      await _v.openai.check(_v.settings.value.sttModel);
      final SpeakResult r = await _v.speech.say('Listo. La voz funciona.');
      if (r == SpeakResult.failed) {
        result = 'La llave funciona, pero la voz falló: ${_v.speech.lastError?.userMessage ?? 'error desconocido'}';
      } else {
        ok = true;
        result = 'La llave funciona: dictado y voz listos.';
      }
    } on OpenAiException catch (e) {
      result = e.kind == OpenAiErrorKind.notFound
          ? 'La llave funciona, pero tu cuenta no tiene acceso a ${_v.settings.value.sttModel}. '
              'Elige otro modelo de dictado.'
          : e.userMessage;
    }
    if (!mounted) return;
    setState(() {
      _testing = false;
      _testOk = ok;
      _testResult = result;
    });
  }

  Future<void> _remove() async {
    final bool? ok = await showDialog<bool>(
      context: context,
      builder: (BuildContext c) => AlertDialog(
        backgroundColor: Orbita.panel,
        title: const Text('¿Quitar la llave de OpenAI?'),
        content: const Text('El dictado y la lectura en voz alta dejan de funcionar hasta que '
            'guardes otra.'),
        actions: <Widget>[
          TextButton(onPressed: () => Navigator.of(c).pop(false), child: const Text('Cancelar')),
          TextButton(
            onPressed: () => Navigator.of(c).pop(true),
            style: TextButton.styleFrom(foregroundColor: Orbita.err),
            child: const Text('Quitar'),
          ),
        ],
      ),
    );
    if (ok != true) return;
    await _v.speech.stop();
    await _v.keys.clear();
    setState(() => _testResult = null);
  }

  Future<void> _set(VoiceSettings Function(VoiceSettings s) f) => _v.settings.update(f);

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge(<Listenable>[_v.keys.status, _v.settings]),
      builder: (BuildContext context, Widget? _) {
        final KeyStatus k = _v.keys.status.value;
        final VoiceSettings s = _v.settings.value;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 20, 16, 4),
              child: Text('Voz', style: TextStyle(color: Orbita.brand, fontWeight: FontWeight.w600)),
            ),
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 0, 16, 8),
              child: Text(
                'Lo que dictas (el audio) y el texto que se lee en voz alta se envían a OpenAI '
                'y se cobran a tu llave. La voz que oyes la genera una IA.',
                style: TextStyle(color: Orbita.dim, fontSize: 13),
              ),
            ),
            ListTile(
              leading: Icon(Icons.key_outlined, color: k.present ? Orbita.done : Orbita.dim),
              title: Text(!k.loaded
                  ? 'Revisando llave…'
                  : k.present
                      ? 'Llave guardada · termina en ••••${k.suffix}'
                      : 'Sin llave de OpenAI'),
              subtitle: k.present
                  ? null
                  : const Text('En tu equipo: qrencode -t ansiutf8 "\$OPENAI_API_KEY" y escanéala.'),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: Wrap(
                spacing: 8,
                runSpacing: 4,
                children: <Widget>[
                  OutlinedButton.icon(
                      onPressed: _scan, icon: const Icon(Icons.qr_code_scanner), label: const Text('Escanear llave')),
                  OutlinedButton.icon(
                      onPressed: _paste, icon: const Icon(Icons.content_paste), label: const Text('Pegar llave')),
                  if (k.present)
                    OutlinedButton.icon(
                      onPressed: _testing ? null : _test,
                      icon: _testing
                          ? const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2))
                          : const Icon(Icons.play_arrow),
                      label: const Text('Probar'),
                    ),
                  if (k.present)
                    OutlinedButton.icon(
                      onPressed: _remove,
                      style: OutlinedButton.styleFrom(foregroundColor: Orbita.err),
                      icon: const Icon(Icons.delete_outline),
                      label: const Text('Quitar llave'),
                    ),
                ],
              ),
            ),
            if (_testResult != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
                child: Text(_testResult!, style: TextStyle(color: _testOk ? Orbita.done : Orbita.err)),
              ),
            const SizedBox(height: 8),
            _choice<SttLanguage>(
              'Idioma del dictado',
              s.language,
              SttLanguage.values,
              (SttLanguage l) => l.label,
              (SttLanguage l) => _set((VoiceSettings x) => x.copyWith(language: l)),
            ),
            _choice<String>(
              'Modelo de dictado',
              s.sttModel,
              kSttModels,
              (String m) => m == kDefaultSttModel ? '$m (recomendado)' : m,
              (String m) => _set((VoiceSettings x) => x.copyWith(sttModel: m)),
            ),
            _choice<String>(
              'Voz',
              s.effectiveVoice,
              s.ttsModel.startsWith('tts-1') ? kTts1Voices : kVoices,
              (String v) => v,
              (String v) => _set((VoiceSettings x) => x.copyWith(voice: v)),
            ),
            _choice<String>(
              'Modelo de voz',
              s.ttsModel,
              kTtsModels,
              (String m) => m,
              (String m) => _set((VoiceSettings x) => x.copyWith(ttsModel: m)),
            ),
            ListTile(
              title: Text('Velocidad de lectura · ${s.speed.toStringAsFixed(1)}×'),
              subtitle: Slider(
                value: s.speed,
                min: VoiceSettings.minSpeed,
                max: VoiceSettings.maxSpeed,
                divisions: 7,
                label: '${s.speed.toStringAsFixed(1)}×',
                onChanged: (double v) => _set((VoiceSettings x) => x.copyWith(speed: v)),
              ),
            ),
            _choice<ReadMode>(
              'Al leer respuestas automáticamente',
              s.autoReadMode,
              ReadMode.values,
              (ReadMode m) => m.label,
              (ReadMode m) => _set((VoiceSettings x) => x.copyWith(autoReadMode: m)),
            ),
            _choice<double>(
              'Terminar el dictado tras un silencio de',
              s.silenceSeconds,
              const <double>[1.5, 2.5, 4.0],
              (double d) => '${d.toStringAsFixed(1)} s',
              (double d) => _set((VoiceSettings x) => x.copyWith(silenceSeconds: d)),
            ),
            SwitchListTile(
              title: const Text('Modo auto: dictar mensajes con OpenAI'),
              subtitle: const Text('Más preciso, pero cuesta. Apagado, el modo auto usa la voz y el '
                  'reconocimiento del teléfono, gratis.'),
              value: s.carCloudDictation,
              onChanged: (bool on) => _set((VoiceSettings x) => x.copyWith(carCloudDictation: on)),
            ),
          ],
        );
      },
    );
  }

  Widget _choice<T>(String title, T value, List<T> options, String Function(T) label, void Function(T) onPick) {
    return ListTile(
      title: Text(title),
      subtitle: Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Wrap(
          spacing: 6,
          runSpacing: 4,
          children: <Widget>[
            for (final T o in options)
              ChoiceChip(
                label: Text(label(o)),
                selected: o == value,
                onSelected: (_) => onPick(o),
              ),
          ],
        ),
      ),
    );
  }
}
