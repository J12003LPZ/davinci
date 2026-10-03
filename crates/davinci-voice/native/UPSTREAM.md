# Native source provenance

whisper.cpp candidate tag: `v1.9.3` (pre-release at plan inspection).
Full peeled revision: `371b5a7561823ab2bb32142d2751e35e7534727b`.
Annotated tag object: `7246b7311e089fe092c4abe7cfad5d0921f8be00`.

Source archive: https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/371b5a7561823ab2bb32142d2751e35e7534727b

Downloaded archive SHA-256:
`89051d8fca516a3ad1f5c2f8f9d2fccb089afbaec338fca3f8731999babc6f81`.

`upstream/` contains unchanged `CMakeLists.txt`, `LICENSE`, `cmake/`,
`include/`, `src/`, and `ggml/` from that archive. No upstream patches.
The embedding build disables examples, tests, services, downloads, dynamic
backends, accelerators and host-specific CPU instructions.

Source licenses remain beside their sources, including `upstream/LICENSE`
and GGML notices. A completed transitive binary redistribution audit and
native sanitizer/platform acceptance remain release requirements.

Upgrade by downloading an immutable archive deliberately, checking its hash,
reviewing native safety changes and backend/ISA defaults, replacing only this
source subset, and running core/native/offline recognition gates. Cargo builds
must never fetch native sources or speech models.
