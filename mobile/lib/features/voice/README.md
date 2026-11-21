# voice

Dictation (mic → text in the composer) and read-aloud, both through the OpenAI
API with the person's own key. Nothing here runs at app start: `registerVoice()`
registers empty forwarders; `VoiceServices` is built on first use.

| File | What |
|---|---|
| `voice.dart` | hooks (`VoiceInput`, `Speaker`), `registerVoice()`, `openVoiceSettings()` |
| `voice_services.dart` | lazy wiring of everything below |
| `openai_client.dart` | the only HTTP to `https://api.openai.com/v1` (hard-coded) |
| `openai_key_store.dart` | key in flutter_secure_storage only; UI sees `••••abcd` |
| `dictation.dart` | `DictationController`: record, auto-stop, transcribe, retry, keep the audio until the text is safe |
| `recorder.dart` | `record` package (AAC-LC .m4a, mono, 16 kHz, 32 kbps) + `SilenceDetector` |
| `speech.dart`, `audio_out.dart`, `tts_cache.dart` | read-aloud queue (prefetch), just_audio + audio focus, 20 MB LRU |
| `text_prep.dart` | markdown → speakable text, read modes, chunking (pure) |
| `reply_watcher.dart` | "a reply completed" detection, read once (pure) |
| `chat_voice.dart` | composer mic + strip, "Callar" button, "Leer respuestas nuevas" |
| `voice_settings*.dart` | preferences (`voice_settings.json`) and Ajustes › Voz |

## OpenAI endpoints (checked against developers.openai.com on 2026-11-20)

- `POST /v1/audio/transcriptions`, multipart: `file` (dictado.m4a, audio/mp4),
  `model` (default `gpt-transcribe`), `prompt`, and for `gpt-transcribe`
  `keywords[]` (git, commit, push, deploy, Claude, Codex, tmux, Flutter, API…) +
  `languages[]=es|en` (omitted on "Automático"). Fallback models
  `gpt-4o-mini-transcribe` / `whisper-1` get `language=` and the vocabulary in
  `prompt` instead. Response `{"text": …}`.
- `POST /v1/audio/speech`, JSON: `model` (`gpt-4o-mini-tts`, fallback `tts-1`),
  `input` (≤ 4096 chars; we send ≤ 1200), `voice` (13 built-ins, default
  `marin`), `response_format: mp3`, `instructions` (not for tts-1). Speed is
  applied by the player, not the API, so changing it never re-synthesises.
- `GET /v1/models/{model}` to test a key (free), then a short spoken phrase.

**Deadlines:** OpenAI deprecated every `/v1/audio/speech` model (`tts-1`,
`tts-1-hd`, `gpt-4o-mini-tts`): shutdown **2027-01-06**; the replacement is
`gpt-realtime-2.1-mini` over the Realtime API (a different workflow, not this
endpoint). `gpt-4o-*-transcribe` and `whisper-1` shut down 2027-02-26
(`gpt-transcribe`, our default, is the replacement). Read-aloud needs a
Realtime-based `Synthesizer` before 2027-01-06; `SpeechService` only depends on
`Synthesizer = Future<Uint8List> Function(text, abort)`.

## Never lose what was said

A recording's meta file is written before recording starts; the audio stays on
disk (app support dir, not cache) until its text has been handed to a sink
that made it durable (composer → draft; Modo auto → outbox or draft). Network
errors / timeouts / 5xx / rate limits retry 3 times (1 s, 3 s); then
"No se pudo transcribir · Reintentar · Descartar", also after a restart.
Orphan audio older than 24 h and pending recordings older than 7 days are
deleted on the first voice use of each process.

`start(owner, requireSpeech: true)` (Modo auto's automatic rounds only): if the
silence detector never heard speech, the recording is deleted without being
transcribed (`DictationNothing`), so silence or road noise is never sent. A
tapped dictation always transcribes (a quiet voice is not discarded). The
earcon gives audio focus back right after playing, so music comes back up.

`audio_session` is imported transitively (it is just_audio's dependency) to
configure focus; declare it in pubspec.yaml to make that explicit.
