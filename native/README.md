# Native runtime boundaries

## WASM Micro Runtime

The Rust `Guest` owns one execution environment, instance, and module. They are
destroyed in that order, including after partial initialization. The loader may
mutate the owned WASM binary. Rust keeps the original allocation until module
unload whenever `wasm_runtime_is_underlying_binary_freeable` returns false.
Function handles borrow their instance. The runtime and native import table
live for the process lifetime.

Each exchange has exclusive access to the guest. The interpreter can move
between host threads when idle. WAMR is built without guest threads, and its
imports run synchronously on the calling thread. The execution environment
stores an opaque pointer to the Rust call state during the export calls. A drop
guard clears it on normal return, errors, and Rust unwinding. Imports must
return before the guest resumes and must never reenter that guest. A Rust panic
inside an `extern "C"` import aborts the process.

Guest addresses are offsets. The bridge validates each nonempty range before
converting it to a native pointer. The guest may relocate linear memory during
an export, so Rust resolves the reply after the final export. Host and guest
buffers are separate allocations when copying. Import metadata and input may
overlap and are read through temporary shared slices. The host owns the parsed
metadata and any bytes it keeps after `execute` returns.

## TensorFlow Lite Micro

The C++ detector owns its tensor arena, frontend, and saved warm state. Rust
owns the detector and holds its immutable FlatBuffer storage through native
destruction. The model base address is aligned to 16 bytes for FlatBuffers and
tensor buffers. An aligned model is borrowed or moved into the detector owner
without a copy. Rust copies misaligned input into aligned storage once at load.
The inference call borrows exactly one PCM frame for its duration and may
mutate the interpreter and arena. Reset and statistics also use exclusive
access to the detector. Multiple detectors share the operation resolver and FFT
tables, which are initialized under the loader lock before inference.

## Versions and verification

WAMR is pinned in `wamr/revision`. TFLite Micro and LiveKit Wake Word are pinned
in `scripts/build-wake-tflite.py`. That TFLite Micro revision pins FlatBuffers
25.9.23 by archive hash in its `MODULE.bazel`. Check these native dependencies
and the local patches separately when reviewing upstream advisories. They are
outside the Rust dependency graph in `Cargo.lock`.

On x86-64 GNU/Linux, build the embedded panel and WASM fixtures as described in
the [build instructions](../guides/development.md#run-checks), then run `bash
scripts/test-native-boundaries.sh` with GCC, G++, CMake, and Ninja available.
The script instruments the native sources and bridges with ASan and UBSan and
runs the Rust integration tests against them. All Goodwin builds use WAMR's
portable bytecode access path to avoid unaligned typed loads and stores. A local
patch aligns table instances and interpreter frames, and uses byte copies to
rewrite packed instruction labels. WAMR's common runtime source keeps
stack locals at their real addresses for its native stack check during ASan
runs. The script verifies the published INT8 model against the same SHA-256 used
for the ESP build.
Instrumented archives are kept in separate build directories.
