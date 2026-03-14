# Research: TTS in Rust — Ecosystem, Models, and Strategies

## Overview

This document surveys the landscape of text-to-speech in Rust as of March 2026, covering: neural TTS models suitable for CPU-only inference, Rust crates for model inference and audio playback, integration strategies, and trade-offs. The goal is to inform future implementation decisions for integrating TTS into a Rust application.

---

## 1. Neural TTS Models (CPU-Friendly)

### 1.1 Pocket TTS (Kyutai Labs)

**Architecture**: Two-stage pipeline — Flow Language Model (FlowLM) generates latent codes autoregressively, Mimi audio codec decodes latents to waveform.

- **Parameters**: ~100M total (~70M FlowLM transformer + ~10M MLP sampler + ~20M Mimi codec). The ONNX FP32 total of 475MB at 4 bytes/param ≈ 119M parameters, consistent with the ~100M claim.
- **Output**: 24kHz mono float32 audio
- **Frame rate**: 12.5 Hz (each frame = 1,920 audio samples)
- **Inference**: CPU-only by design (GPU gives zero speedup per Kyutai's blog post). Uses Lagrangian Self Distillation (LSD) decoding with ODE solver.
- **Tokenization**: SentencePiece (no espeak-ng dependency)
- **Voice support**: 8 built-in voices (alba, marius, javert, jean, fantine, cosette, eponine, azelma) + zero-shot voice cloning from reference audio
- **Languages**: Multilingual (trained on multilingual data)
- **License**: MIT (code), CC-BY-4.0 (weights)
- **Native format**: SafeTensors (PyTorch)

**ONNX export**: [KevinAHM/pocket-tts-onnx-export](https://github.com/KevinAHM/pocket-tts-onnx-export) provides complete ONNX export. Models hosted at [KevinAHM/pocket-tts-onnx](https://huggingface.co/KevinAHM/pocket-tts-onnx):

| Model file | FP32 | INT8 | Role |
|-----------|------|------|------|
| `text_conditioner.onnx` | 16 MB | — | Text tokens → embeddings |
| `flow_lm_main.onnx` | 303 MB | 76 MB | Autoregressive latent generation |
| `flow_lm_flow.onnx` | 39 MB | 10 MB | ODE solver (Euler steps) |
| `mimi_decoder.onnx` | 42 MB | 23 MB | Latents → audio waveform |
| `mimi_encoder.onnx` | 73 MB | — | Audio → latents (voice cloning) |
| **Total** | **475 MB** | **~200 MB** | |

**Inference complexity**: The ONNX version requires orchestrating 5 separate model files with **74 state tensors** (18 for FlowLM KV caches + 56 for Mimi decoder state). The autoregressive loop generates one 32-dim latent per step, runs an ODE solver (1-10 Euler steps per latent), and batch-decodes latents through Mimi every N frames. This is well-documented in the export repo's Python reference implementation and a JavaScript implementation in the HuggingFace Space.

**Key insight**: The complexity of Pocket TTS inference is not in any single operation (each is straightforward — matrix multiply, Euler step, Conv1d) but in the **orchestration**: correctly threading 74 named state tensors between calls, managing KV cache slicing, and coordinating the autoregressive loop with the decoder.

### 1.2 Kokoro (hexgrad)

**Architecture**: Single-stage model — text phonemes → audio waveform directly via neural vocoder.

- **Parameters**: 82M
- **Output**: 24kHz mono float32
- **Inference**: CPU real-time (3-11x real-time factor on modern CPUs; 40-70ms per sentence on GPU)
- **Tokenization**: Primary G2P engine is Misaki (hexgrad/misaki) with gold/silver phoneme dictionaries; espeak-ng is a fallback for out-of-vocabulary words. The model was trained on these phonemes, making some form of espeak-ng a hard dependency.
- **Voice support**: Multiple preset voices (`af_heart`, `af_nicole`, etc.) stored as 256-float style vectors. 54 voices across 8 languages. Voice blending supported (e.g., `af_sky.4 + af_nicole.5`).
- **Languages**: 8 languages natively (American English, British English, French, Japanese, Korean, Mandarin, Hindi, Spanish). Note: individual Rust crates may not expose the full multilingual capability (see Section 4).
- **License**: Apache 2.0 (code + weights)
- **Format**: Single ONNX file

**Model files**: [onnx-community/Kokoro-82M-v1.0-ONNX](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX):

| Variant | Size | Notes |
|---------|------|-------|
| `model.onnx` (FP32) | 326 MB | Best quality |
| `model_fp16.onnx` | 163 MB | Balanced |
| `model_quantized.onnx` (INT8) | 92 MB | Lightweight |
| `model_q8f16.onnx` | 86 MB | Smallest |

Plus `tokenizer.json` (~2 KB) and voice `.bin` files (~1 KB each).

**Inference complexity**: Vastly simpler than Pocket TTS — single ONNX model, 3 inputs (`input_ids: i64`, `style: f32`, `speed: f32`), one output (audio samples). No state tensors, no autoregressive loop, no ODE solver.

### 1.3 Piper TTS

**Architecture**: VITS-based (Variational Inference with adversarial learning for end-to-end TTS).

- **Parameters**: Varies by voice (typically 20-60M)
- **Output**: 16-22kHz depending on voice model
- **Inference**: Very fast on CPU, designed for embedded/edge deployment
- **Format**: ONNX natively (designed for ONNX Runtime deployment)
- **Voices**: Large community voice collection (100+ voices, 30+ languages)
- **License**: MIT
- **Quality**: Noticeably below Kokoro and Pocket TTS — more robotic/synthetic sounding

### 1.4 Other Notable Models

- **Qwen3-TTS**: Streaming-native design with `synthesize_streaming()`. Time-to-first-audio: 444-580ms. Has a Rust implementation (`qwen3-tts-rs`). Larger model, higher resource requirements.
- **Kitten TTS (KittenML)**: 15M params, runs on Raspberry Pi, CPU-only. Very lightweight but lower quality.
- **VoiRS**: Pure Rust neural TTS (no ONNX, no FFI). Alpha status (v0.1.0-alpha.2). Supports streaming, WASM, and GPU via wgpu. Worth watching but not production-ready.
- **TADA (Hume AI)**: 1B params, fastest LLM-based TTS (RTF 0.09), zero hallucinations. Too large for CPU-only.

---

## 2. How Streaming Works in Neural TTS

There are three levels of TTS streaming, from coarsest to finest:

### Level 1: Sentence-chunking (application layer, works with any model)

The application splits text into sentences and pipelines synthesis with playback. This is the **universal strategy** that works regardless of model architecture:

1. Split input text into sentences (or short paragraphs)
2. Feed sentence 1 to the model → receive audio for sentence 1
3. Start playing sentence 1 audio immediately
4. While sentence 1 plays, feed sentence 2 to the model
5. When sentence 1 finishes, sentence 2 audio is ready → seamless transition
6. Repeat until all sentences are spoken

**Time-to-first-audio** depends on how fast the model synthesizes one short sentence:
- Kokoro: ~100-300ms per short sentence on CPU
- Pocket TTS: ~200-500ms per short sentence on CPU
- Piper: ~50-150ms per short sentence on CPU

All of these are well below the threshold where a human perceives a delay. The pipeline creates the illusion of continuous streaming speech.

**Progress callbacks**: Some implementations (e.g., sherpa-onnx) provide progress callbacks during synthesis. These may or may not deliver partial audio samples. If they do, sub-sentence streaming becomes possible — start playing audio before the full sentence is done. This is model-dependent and needs verification per implementation.

**Pocket TTS native streaming**: The Python Pocket TTS library has a true `generate_audio_stream()` that yields audio chunks as the autoregressive loop generates them (every 3 frames initially, then every 12 frames). Each chunk is 1,920 samples × N frames. This is finer-grained than sentence-chunking but requires access to the autoregressive loop internals — only possible with the raw `ort` approach, not through sherpa-onnx's batch API.

---

## 3. Rust Crates for ONNX Inference

### 3.1 `ort` (pykeio/ort)

The primary Rust binding for Microsoft's ONNX Runtime. Production-grade, actively maintained.

- **Version**: v2.0.0-rc.12 (wrapping ONNX Runtime 1.24.3)
- **API**: Safe Rust wrappers around C API. `Session::run()` with named inputs/outputs via `inputs!` macro.
- **Thread safety**: `Session` is `Send + Sync`
- **Features**: `load-dynamic` (runtime load of `libonnxruntime.so`) or `download-binaries` (auto-download at build)
- **Execution providers**: CPU, CUDA, TensorRT, OpenVINO, CoreML, QNN
- **Stateful inference**: Supported via passing state tensors as named inputs and extracting updated states from outputs. No special API needed — just pass the tensors through.
- **Used by**: HuggingFace Text Embeddings Inference, Google Magika, edge-transformers

**Relevance**: Direct dependency if implementing Pocket TTS manually. Transitive dependency of `kokoroxide` and `sherpa-onnx`.

### 3.2 `sherpa-onnx` (k2-fsa/sherpa-onnx)

Unified speech processing library with official Rust bindings. Wraps a C++ backend that handles all model orchestration internally.

- **Version**: v0.1.10+ (Rust bindings), backed by sherpa-onnx C++ v1.12.25+
- **TTS models supported**: Pocket TTS, Kokoro, VITS, Matcha, Kitten, Zipvoice, Supertonic (7+ models)
- **API pattern**: Config struct → `OfflineTts::create(&config)` → `tts.generate(text)` or `tts.generate_with_config(text, &gen_config, callback)`
- **Progress callbacks**: `|samples: &[f32], progress: f32| -> bool` — receives partial progress, return `false` to cancel
- **Thread safety**: Implemented `Send + Sync`
- **Build**: Requires CMake + Clang at compile time (builds C++ from source or uses pre-built static libs)
- **Binary size**: 30-50MB with static linking
- **Also supports**: Speech recognition (ASR), speaker diarization, keyword spotting, audio tagging, voice activity detection

**Note**: `sherpa-rs` (thewh1teagle/sherpa-rs) is **deprecated** as of its latest commit. The official k2-fsa/sherpa-onnx Rust API is the maintained successor.

**Key advantage**: Handles all Pocket TTS inference complexity (5 ONNX models, 74 state tensors, ODE solver, KV cache management) in battle-tested C++. You just pass text and get audio back.

---

## 4. Rust Crates for TTS (Higher-Level)

### 4.1 `kokoroxide`

[github.com/dhruv304c2/kokoroxide](https://github.com/dhruv304c2/kokoroxide) — Rust Kokoro TTS wrapper using `ort`.

- **Version**: v0.1.5 (WIP)
- **ort version**: v1.16 (older; current is v2.0)
- **API**: `KokoroTTS::with_config(config)` → `tts.speak(text, &voice)` → `GeneratedAudio { samples: Vec<f32>, sample_rate: u32 }`
- **Streaming**: None — full utterance only
- **Languages**: English only (Misaki phoneme conversion hardcoded for US English)
- **espeak-ng**: Required. Custom FFI bindings (`#[link(name = "espeak-ng")]`). Link-time failure if missing.
- **Phoneme pipeline**: Text → espeak-ng IPA → Misaki phonemes → token IDs → ONNX model
- **Voice format**: `.bin` files containing 256-float style vectors per voice
- **License**: MIT

**Assessment**: Simplest integration path for Kokoro-only TTS. Limited by English-only, no streaming, WIP quality, and old ort version.

### 4.2 `kokorox`

[github.com/byteowlz/kokorox](https://github.com/byteowlz/kokorox) — more mature Kokoro implementation.

- **Version**: v0.2.3+
- **ort version**: v2.0.0-rc.11
- **Streaming**: Yes — WebSocket streaming, pipe mode for LLM integration
- **Languages**: 6+ via `espeak-rs`
- **Features**: OpenAI-compatible `/v1/audio/speech` API server, voice blending, language detection
- **espeak-ng**: Required via `espeak-rs` crate
- **License**: **GPL 3.0** (due to `espeak-rs-sys` static linking)

**Assessment**: Most feature-complete Kokoro implementation. The GPL license is the critical concern — it infects any statically linked binary. The streaming and multi-language support are strong, but it's designed as a standalone application, not a library.

### 4.3 `piper-rs`

[github.com/thewh1teagle/piper-rs](https://github.com/thewh1teagle/piper-rs) — Rust bindings for Piper TTS via `ort`.

- **ort version**: v1.22 (ONNX Runtime 1.22)
- **API**: Load model + config JSON, synthesize text
- **Quality**: Lower than Kokoro — Piper uses older VITS architecture
- **License**: MIT

### 4.4 System TTS crates

- **`tts` crate** (v0.26.3): Wraps OS speech APIs (SAPI/WinRT on Windows, Speech Dispatcher on Linux, AVFoundation on macOS/iOS). Not neural, quality varies by OS. No model download needed.
- **`msedge-tts`**: MSEdge Read Aloud API (requires network)
- **`aspeak`**: Azure TTS API client (requires network + API key)

These are irrelevant for CPU-only neural TTS but noted for completeness.

---

## 5. Rust Crates for Audio Output

### 5.1 `cpal` (RustAudio/cpal)

Cross-platform audio I/O library. The standard for real-time audio in Rust.

- **Model**: Callback-based. The audio device calls your closure `|data: &mut [f32]|` on a high-priority thread to fill the output buffer.
- **Latency**: Configurable. Default ~10ms. Can go lower with tuning.
- **Thread model**: Callback runs on dedicated OS thread with real-time priority (via rtkit on Linux).
- **Platforms**: Linux (ALSA/PulseAudio/JACK), macOS (CoreAudio), Windows (WASAPI), Android, iOS, Emscripten
- **Usage pattern**: Create output stream → callback pulls samples from a shared ring buffer → write silence on underrun

**Best for**: Low-latency streaming playback where you control timing. Ideal for TTS streaming pipeline.

### 5.2 `rodio` (RustAudio/rodio)

Higher-level audio playback built on cpal.

- **Model**: Source-based. Implement `rodio::Source` trait (an `Iterator<Item = f32>`) and rodio handles mixing and playback.
- **Default latency**: 100ms (adjustable to 512-1024 samples for lower latency)
- **Thread model**: Spawns background mixer thread
- **Simpler API** but less control than cpal

**Best for**: Simple playback scenarios where latency isn't critical. Less ideal for real-time streaming synthesis.

### 5.3 Ring buffer options

For bridging between synthesis thread and audio callback:

- **`ringbuf` crate**: Lock-free SPSC ring buffer. Zero-copy, wait-free. Ideal for audio.
- **`VecDeque<f32>` + `parking_lot::Mutex`**: Simpler, lock-based. Sub-microsecond hold time. Good enough for TTS where the callback just copies samples.
- **`crossbeam::channel`**: MPSC channel. Higher overhead than ring buffer but simpler synchronization.

---

## 6. Integration Strategies

### Strategy A: sherpa-onnx as unified backend

Use official sherpa-onnx Rust bindings. All model complexity handled by C++ backend.

**Flow**: Text → sentence split → sherpa-onnx `generate()` per sentence → audio samples → cpal playback

**Pros**:
- Supports Pocket TTS + Kokoro + 5 other models via config change
- All ONNX orchestration, state management, ODE solver in proven C++
- Single dependency for all TTS models
- Apache 2.0 license

**Cons**:
- CMake + Clang build dependency
- 30-50MB binary size increase
- C++ FFI layer (potential for hard-to-debug segfaults)
- No true sub-sentence streaming (batch synthesis per call, though progress callbacks may provide partial audio)
- Opaque — can't optimize or modify inference internals

**Build requirement**: `cmake`, `clang` (or pre-built static libraries)

### Strategy B: Raw `ort` with Pocket TTS ONNX models

Use `ort` crate directly. Implement all inference orchestration in Rust.

**Flow**: Text → SentencePiece tokenize → text_conditioner → autoregressive FlowLM loop → ODE solver → Mimi decoder → cpal playback

**Pros**:
- Full control over inference pipeline
- Can implement true sub-sentence streaming (pipe Mimi decoder output directly to playback as frames are generated)
- No C++ build dependency beyond ONNX Runtime shared lib
- Can optimize hot paths (e.g., skip ODE steps for faster inference, tune KV cache sizes)

**Cons**:
- 10-14 days implementation effort (realistic estimate from detailed walkthrough)
- 74 state tensors to manage correctly — high debugging risk
- SentencePiece C++ FFI still needed for tokenization
- Must maintain inference code as Pocket TTS evolves

**The unique advantage**: This is the only strategy that enables true sub-sentence streaming — audio can start playing after just 3 latent frames (~240ms of audio), not waiting for a full sentence. This matters for very long sentences or when minimal latency is critical.

### Strategy C: `kokoroxide` for Kokoro-only

Use the existing Rust crate. Simplest path.

**Flow**: Text → espeak-ng phonemes → Kokoro ONNX model → audio samples → cpal playback

**Pros**:
- 3-5 days integration
- Single ONNX model, no state management
- Simple API

**Cons**:
- English only
- espeak-ng system dependency
- WIP crate quality (v0.1.5)
- Cannot use Pocket TTS model
- No sub-sentence streaming

### Strategy D: Trait abstraction with pluggable backends

Define a `TtsBackend` trait and implement multiple backends behind it.

```
trait TtsBackend: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str) -> Result<AudioChunk>;
    fn sample_rate(&self) -> u32;
}
```

The engine layer handles text chunking, sentence-level streaming, and audio playback regardless of backend. Backends are swappable at configuration time.

This is an architectural pattern, not a standalone strategy — it combines with any of A/B/C above. The question is whether the abstraction overhead is justified:

- **Yes if**: multiple backends are expected (e.g., Pocket TTS for quality, Kokoro for speed, system TTS as fallback)
- **No if**: only one backend will ever be used (YAGNI)

---

## 7. espeak-ng: The Kokoro Tax

Any Kokoro-based approach (kokoroxide, kokorox, sherpa-onnx with Kokoro model) requires espeak-ng for text → phoneme conversion. This is a hard dependency baked into the model's training data.

**What it is**: espeak-ng is a C library (~2MB) that converts text to IPA phonemes. It supports 100+ languages.

**Integration methods in Rust**:
- `espeakng-sys`: Raw FFI bindings (C compilation at build time)
- `espeakng`: Safe wrapper (higher-level API)
- `espeak-rs`: Another wrapper (used by kokorox, triggers GPL via static linking)
- Custom FFI: kokoroxide writes its own `#[link(name = "espeak-ng")]` bindings

**System dependency**: Must be installed via package manager (`pacman -S espeak-ng`, `apt install espeak-ng libespeak-ng-dev`). Cannot be bundled as a Rust crate dependency.

**Pocket TTS does NOT need espeak-ng** — it uses SentencePiece tokenization directly on text. This is a meaningful differentiator.

---

## 8. Model Download and Distribution

All neural TTS models require downloading model files (86MB - 475MB). Strategies:

1. **Manual download**: User downloads from HuggingFace, places in a known directory. Simplest, most explicit, requires documentation.
2. **Auto-download on first use**: Application detects missing models, downloads via HTTPS (reqwest). Better UX, adds complexity and network dependency.
3. **`hf-hub` crate**: Rust HuggingFace Hub client. Handles caching, ETags, resumable downloads. Used by kokorox.
4. **System package**: Some models available via OS package managers (rare for TTS).

All approaches need a model cache directory (e.g., `~/.cache/<app>/models/`) and graceful handling of missing files.

---

## 9. Comparison Summary

| Dimension | Pocket TTS (sherpa-onnx) | Pocket TTS (raw ort) | Kokoro (kokoroxide) | Kokoro (kokorox) |
|-----------|-------------------------|---------------------|--------------------|-----------------|
| **Quality** | High (100M) | High (100M) | High (82M) | High (82M) |
| **Model size** | ~200MB INT8 | ~200MB INT8 | 86-326MB | 86-326MB |
| **Streaming** | Sentence-chunking | Sub-sentence possible | Sentence-chunking | Native WebSocket |
| **Languages** | Multilingual | Multilingual | English only | 6+ languages |
| **espeak-ng** | No | No | Yes | Yes |
| **Build deps** | CMake + Clang | ONNX Runtime lib | espeak-ng lib | espeak-ng lib |
| **Integration effort** | Medium (5-7 days) | High (10-14 days) | Low (3-5 days) | Medium (5-7 days) |
| **Risk** | Low-Medium | High | Low | Medium (GPL) |
| **Future models** | Any (7+ supported) | Pocket TTS only | Kokoro only | Kokoro only |
| **License** | Apache 2.0 | MIT + CC-BY-4.0 | MIT | **GPL 3.0** |
| **Maturity** | Production (C++ core) | N/A (custom) | WIP (v0.1.5) | Stable (v0.2.3) |

---

## 10. Open Questions for Implementation

1. **sherpa-onnx progress callback**: Does the `|samples: &[f32], progress: f32|` callback deliver actual audio samples that can be played immediately? If so, sub-sentence streaming is possible even through sherpa-onnx. Needs empirical verification.

2. **sherpa-onnx binary size**: What is the actual binary size impact when statically linking sherpa-onnx? The 30-50MB estimate is from sherpa-rs — the official bindings may differ.

3. **cpal vs rodio**: For the specific use case of "push audio chunks from a synthesis thread to speakers," is cpal's lower-level API worth the complexity over rodio's simpler source-based model?

4. **Model quality comparison**: No direct A/B comparison between Pocket TTS and Kokoro on the same text exists in the research. An empirical listening test would be valuable before committing to a model.

5. **SentencePiece in Rust**: The `sentencepiece` crate (FFI) vs `tokenizers` crate (pure Rust, needs model conversion) — which produces identical tokenization to the Python reference? Critical for Pocket TTS quality.

6. **Build time impact**: How much does sherpa-onnx's CMake compilation add to `cargo build` time? For a developer workflow, >60s incremental builds would be painful.

---

## References

- [Pocket TTS blog post (Kyutai, Jan 2026)](https://kyutai.org/blog/2026-01-13-pocket-tts)
- [KevinAHM/pocket-tts-onnx-export](https://github.com/KevinAHM/pocket-tts-onnx-export) — ONNX export + reference inference
- [KevinAHM/pocket-tts-onnx](https://huggingface.co/KevinAHM/pocket-tts-onnx) — ONNX model files
- [ort crate](https://github.com/pykeio/ort) — Rust ONNX Runtime bindings
- [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) — Official Rust bindings (in `rust-api/` dir)
- [kokoroxide](https://github.com/dhruv304c2/kokoroxide) — Rust Kokoro wrapper
- [kokorox](https://github.com/byteowlz/kokorox) — Mature Kokoro Rust implementation
- [onnx-community/Kokoro-82M-v1.0-ONNX](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) — Kokoro ONNX models
- [cpal](https://github.com/RustAudio/cpal) — Cross-platform audio library
- [rodio](https://github.com/RustAudio/rodio) — Audio playback
- [piper-rs](https://github.com/thewh1teagle/piper-rs) — Piper TTS Rust bindings
- [qwen3-tts-rs](https://github.com/TrevorS/qwen3-tts-rs) — Streaming TTS example
- [VoiRS](https://crates.io/crates/voirs) — Pure Rust neural TTS (alpha)
