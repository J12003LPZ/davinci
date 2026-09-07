#include "voice_shim.h"
#include "whisper.h"
#include <cstring>
#include <memory>

static void quiet(enum ggml_log_level, const char *, void *) {}

extern "C" void *dv_load(void *bytes, size_t size) {
    if (!bytes || size < 16) return nullptr;
    try {
        whisper_log_set(quiet, nullptr);
        auto params = whisper_context_default_params();
        params.use_gpu = false;
        return whisper_init_from_buffer_with_params_no_state(bytes, size, params);
    } catch (...) { return nullptr; }
}

extern "C" void dv_free(void *context) {
    try { if (context) whisper_free(static_cast<whisper_context *>(context)); }
    catch (...) {}
}

extern "C" int dv_decode(void *context, const float *pcm, size_t samples,
    const char *language, int threads, dv_abort abort, void *user, char *text, size_t capacity) {
    if (!context || !pcm || samples < 4800 || samples > 1920000 || !language ||
        !text || capacity < 1 || threads < 1 || threads > 4 || !abort) return 2;
    text[0] = '\0';
    try {
        auto *ctx = static_cast<whisper_context *>(context);
        std::unique_ptr<whisper_state, decltype(&whisper_free_state)> state(whisper_init_state(ctx), whisper_free_state);
        if (!state || abort(user)) return 1;
        auto params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
        params.n_threads = threads;
        params.translate = false;
        params.no_context = true;
        params.no_timestamps = true;
        params.print_special = false;
        params.print_progress = false;
        params.print_realtime = false;
        params.print_timestamps = false;
        params.temperature = 0;
        params.temperature_inc = 0;
        params.greedy.best_of = 1;
        params.language = language;
        // "auto" selects a language and continues decoding. detect_language=true
        // instead returns immediately after detection, without a transcript.
        params.detect_language = false;
        if (std::strcmp(language, "auto") != 0 && whisper_lang_id(language) < 0) return 2;
        params.abort_callback = abort;
        params.abort_callback_user_data = user;
        if (whisper_full_with_state(ctx, state.get(), params, pcm, static_cast<int>(samples)) != 0 || abort(user)) return 1;
        size_t used = 0;
        for (int i = 0; i < whisper_full_n_segments_from_state(state.get()); ++i) {
            if (whisper_full_get_segment_no_speech_prob_from_state(state.get(), i) > params.no_speech_thold) continue;
            const char *segment = whisper_full_get_segment_text_from_state(state.get(), i);
            if (!segment) return 1;
            size_t len = std::strlen(segment);
            if (len >= capacity - used) { text[0] = '\0'; return 3; }
            std::memcpy(text + used, segment, len);
            used += len;
        }
        text[used] = '\0';
        return 0;
    } catch (...) { text[0] = '\0'; return 1; }
}
