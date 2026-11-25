#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#if defined(BH_PLATFORM_ESP_IDF)
#include "esp_heap_caps.h"
#endif
#include "wasm_export.h"

extern int32_t goodwin_wamr_host_call(wasm_exec_env_t, uint32_t, uint32_t, uint32_t, uint32_t);
extern int32_t goodwin_wamr_host_result(wasm_exec_env_t, uint32_t, uint32_t);
extern void goodwin_wamr_signal(wasm_exec_env_t);

typedef union {
    size_t size;
    uint64_t alignment;
} Allocation;

static void *allocate(mem_alloc_usage_t usage, unsigned size)
{
    (void)usage;
    const size_t bytes = sizeof(Allocation) + (size ? size : 1);
    if (bytes < size)
        return NULL;
#if defined(BH_PLATFORM_ESP_IDF)
    Allocation *allocation = heap_caps_aligned_calloc(
        8, 1, bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
#else
    Allocation *allocation = calloc(1, bytes);
#endif
    if (!allocation)
        return NULL;
    allocation->size = size;
    return allocation + 1;
}

static void release(mem_alloc_usage_t usage, void *pointer)
{
    (void)usage;
    if (!pointer)
        return;
#if defined(BH_PLATFORM_ESP_IDF)
    heap_caps_free((Allocation *)pointer - 1);
#else
    free((Allocation *)pointer - 1);
#endif
}

static void *resize(mem_alloc_usage_t usage, bool full_size_mapped, void *pointer, unsigned size)
{
    (void)full_size_mapped;
    void *next = allocate(usage, size);
    if (pointer && next) {
        const size_t old = ((Allocation *)pointer - 1)->size;
        memcpy(next, pointer, old < size ? old : size);
        release(usage, pointer);
    }
    return next;
}

bool goodwin_wamr_init(void)
{
    static NativeSymbol symbols[] = {
        { "host_call", goodwin_wamr_host_call, "(iiii)i", NULL },
        { "host_result", goodwin_wamr_host_result, "(ii)i", NULL },
        { "play_timer_signal", goodwin_wamr_signal, "()", NULL },
    };
    RuntimeInitArgs args = {
        .mem_alloc_type = Alloc_With_Allocator,
        .mem_alloc_option.allocator = {
            .malloc_func = allocate,
            .realloc_func = resize,
            .free_func = release,
        },
        .native_module_name = "goodwin",
        .native_symbols = symbols,
        .n_native_symbols = sizeof(symbols) / sizeof(*symbols),
    };
    return wasm_runtime_full_init(&args);
}

wasm_module_t goodwin_wamr_load(uint8_t *data, uint32_t size, char *error, uint32_t capacity)
{
    const LoadArgs args = { .wasm_binary_freeable = WASM_ENABLE_FAST_INTERP };
    return wasm_runtime_load_ex(data, size, &args, error, capacity);
}

wasm_module_inst_t goodwin_wamr_instantiate(wasm_module_t module, uint32_t stack,
                                           uint32_t pages, char *error, uint32_t capacity)
{
    const InstantiationArgs args = {
        .default_stack_size = stack,
        .host_managed_heap_size = 0,
        .max_memory_pages = pages,
    };
    return wasm_runtime_instantiate_ex(module, &args, error, capacity);
}

wasm_function_inst_t goodwin_wamr_lookup(wasm_module_inst_t instance, const char *name, uint32_t params)
{
    wasm_function_inst_t function = wasm_runtime_lookup_function(instance, name);
    if (!function || params > 1
        || wasm_func_get_param_count(function, instance) != params
        || wasm_func_get_result_count(function, instance) != 1)
        return NULL;
    wasm_valkind_t type;
    if (params) {
        wasm_func_get_param_types(function, instance, &type);
        if (type != WASM_I32)
            return NULL;
    }
    wasm_func_get_result_types(function, instance, &type);
    return type == WASM_I32 ? function : NULL;
}
