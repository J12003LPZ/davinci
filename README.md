# Claude Code / DaVinci research export

Download this branch using **Code → Download ZIP**, then extract it.

- **claude-code-research-reports.zip**: standalone smaller archive with reports, scripts, evidence and analyzed JavaScript. Open this directly to read the findings.
- **claude-code-davinci-research.zip.part01–part04**: the complete original archive, split only to meet GitHub's file-size limit. Includes research artifacts, native executable, original skill document and pinned DaVinci source snapshot.

To reconstruct the complete ZIP, run `python3 reassemble.py` (Windows: `py reassemble.py`) from the extracted directory. The script verifies the complete archive's SHA-256. The parts are not independently extractable ZIP files.

Alternatively on macOS/Linux:

```sh
cat claude-code-davinci-research.zip.part* > claude-code-davinci-research.zip
```

Full archive SHA-256: `60e2806f7f4bc7de1ddeb9f65339d4a194414e55ffb3abec104dcf7cc25dcaee`.
Individual download checksums are in `SHA256SUMS`.

## Scope

The reports reverse-engineer the published Claude Code **2.1.80 JavaScript** runtime. The installed **2.1.302 native executable** was identified and hashed, not reverse-engineered. Findings distinguish source verification, controlled observations, inference and unknowns. Experiments use simulated providers; they do not establish task-success improvements.

DaVinci comparison source: commit `53e999a7fc24c721c484a710093eeeff611641bf`. This export branch contains research files only; DaVinci's main branch was not modified.
