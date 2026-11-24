#include <stdint.h>

static char buffer[128 * 1024];
static const char response[] = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"status\":200}}";

__attribute__((export_name("goodwin_buffer")))
int32_t goodwin_buffer(void) {
    return (int32_t)(uintptr_t)buffer;
}

__attribute__((export_name("goodwin_call")))
int32_t goodwin_call(int32_t length) {
    for (int32_t i = 0; i + 3 < length; ++i) {
        if (buffer[i] == 'l' && buffer[i + 1] == 'o' && buffer[i + 2] == 'o' && buffer[i + 3] == 'p') {
            for (volatile uint32_t tick = 0; tick < 100000; ++tick) {}
        }
    }
    if (__builtin_wasm_memory_grow(0, 8) != (uintptr_t)-1) {
        __builtin_trap();
    }
    for (unsigned i = 0; i < sizeof(response); ++i) {
        buffer[i] = response[i];
    }
    return sizeof(response) - 1;
}
