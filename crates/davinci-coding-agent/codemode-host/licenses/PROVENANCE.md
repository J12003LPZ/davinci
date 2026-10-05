# Optional Codemode host notices

The runtime bundle includes the JavaScript adapter, Pi's standalone sandbox,
QuickJS WASI JavaScript, and `quickjs.wasm`. It excludes Pi model APIs, npm,
package lifecycle scripts, and the optional QuickJS extension binaries.
Node is a separately installed, explicitly selected runtime; this bundle does
not redistribute Node.

Retained notices come from these immutable sources:

- Pi 1.0.2: `pi-LICENSE`, tag v1.0.2, commit
  `cd32f7725fdbddbaecdff5b1e68491563394e0ca` in the earendil-works/pi repository.
- quickjs-wasi 3.6.2: `quickjs-wasi-LICENSE`, distributed npm package;
  source commit `5a7a0eeda87c99542f8cf3095b6d61ecfa755977` in
  vercel-labs/quickjs-wasi.
- quickjs-ng: `quickjs-ng-LICENSE`, the above source's engine submodule,
  commit `6d46d07d04041b40f4f49eaa7fdebe44c314c699`.
- WASI libc: its top-level and component notices, wasi-sdk-32 tag;
  WASI SDK 32's libc submodule is
  `2fc32bc81b9f07f8d9525edea59bfbaf760c06d6`. `dlmalloc-NOTICE`
  and `emmalloc-NOTICE` retain the copyright/license paragraphs from
  `dlmalloc/src/malloc.c` and `emmalloc/emmalloc.c` at that commit.
- LLVM and compiler-rt: `llvm-LICENSE.TXT` and `compiler-rt-LICENSE.TXT`,
  WASI SDK 32's LLVM submodule commit
  `4434dabb69916856b824f68a64b029c67175e532`.

QuickJS WASI's pinned Makefile requires WASI SDK 32 and links the engine
with WASI libc and compiler runtime support. The component notices accompany
the engine rather than relying only on the npm wrapper's license.
The trusted Rust manifest hashes every notice together with executable assets.
