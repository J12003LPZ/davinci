#pragma once
#include <stddef.h>
#include <stdbool.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef bool (*dv_abort)(void *);
void *dv_load(void *bytes, size_t size);
void *dv_load_file(const char *path);
void dv_free(void *context);
/* Returns 0 on success, 1 on error, 2 for invalid arguments, 3 for text limit. */
int dv_decode(void *context, const float *pcm, size_t samples, const char *language,
              int threads, dv_abort abort, void *user, char *text, size_t capacity);
#ifdef __cplusplus
}
#endif
