import 'package:flutter/material.dart';

import '../../core/platform/native_channel.dart';
import '../../ui/theme.dart';

/// Full-screen QR scanner. Pops with the first QR text it reads, or null
/// when the person goes back.
///
/// The camera and decoding are native (`ScanActivity`: camera preview +
/// ZXing's QR reader), so they need no Play Services download, work offline
/// and add ~0.1 MB instead of ML Kit's ~5.8 MB. This route only launches it.
class QrScanScreen extends StatefulWidget {
  const QrScanScreen({
    super.key,
    this.title = 'Escanea el código de «Remoto»',
    this.fallback = 'No pude abrir la cámara. Vuelve y pega el enlace.',
  });

  /// What is being scanned, shown while the camera opens.
  final String title;

  /// What to do instead when the camera cannot open.
  final String fallback;

  @override
  State<QrScanScreen> createState() => _QrScanScreenState();
}

class _QrScanScreenState extends State<QrScanScreen> {
  String? _problem;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _scan());
  }

  Future<void> _scan() async {
    setState(() => _problem = null);
    final QrScanResult r = await NativeChannel.instance.scanQr();
    if (!mounted) return;
    switch (r) {
      case QrScanned(:final String text):
        Navigator.of(context).pop(text);
      case QrScanFailed(reason: 'cancelled'):
        Navigator.of(context).pop();
      case QrScanFailed(reason: 'denied'):
        setState(() => _problem = 'Sin permiso de cámara. Actívalo en los ajustes de Android, '
            'o vuelve y pega el enlace.');
      case QrScanFailed():
        setState(() => _problem = widget.fallback);
    }
  }

  @override
  Widget build(BuildContext context) {
    final String? problem = _problem;
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(title: Text(widget.title)),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: problem == null
              ? const Text('Abriendo la cámara…', style: TextStyle(color: Orbita.dim, fontSize: 15))
              : Column(
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    Text(problem,
                        textAlign: TextAlign.center,
                        style: const TextStyle(color: Orbita.text, fontSize: 16, height: 1.35)),
                    const SizedBox(height: 20),
                    OutlinedButton(onPressed: _scan, child: const Text('Intentar de nuevo')),
                    TextButton(
                      onPressed: () => NativeChannel.instance.openSettings(),
                      child: const Text('Abrir ajustes de Android'),
                    ),
                  ],
                ),
        ),
      ),
    );
  }
}
