# Offline recognition fixture

`jfk.pcm` is 176,000 mono signed little-endian 16-bit samples at 16 kHz
(11 seconds), extracted without resampling from `samples/jfk.wav` in
whisper.cpp commit `371b5a7561823ab2bb32142d2751e35e7534727b`.
PCM SHA-256: `a29462b8ebd467318000e683b9117ade46230d3255ed2024e7db894abd9b38c9`.

Source: <https://github.com/ggml-org/whisper.cpp/blob/371b5a7561823ab2bb32142d2751e35e7534727b/samples/jfk.wav>.
The excerpt is President John F. Kennedy's 1961 inaugural address, a United
States federal government work in the public domain in the United States.
The enclosing upstream repository is MIT licensed (see `native/upstream/LICENSE`).
The expected English excerpt includes "ask not what your country can do for you".
Tests normalize punctuation and case; they never require exact punctuation.

The inline ignored engine test requires `DAVINCI_VOICE_TEST_MODEL` to point to
an explicitly provisioned approved model. `DAVINCI_VOICE_TEST_MODEL_ID` defaults
to `base`. It never downloads models or opens an audio device. Synthetic silence
is generated in memory. Additional-language, noise and technical-name accuracy
fixtures remain a release acceptance gap.
