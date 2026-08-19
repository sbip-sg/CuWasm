#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CuwasmModule CuwasmModule;

typedef struct CuwasmMailbox {
    uint32_t fn_id;
    uint16_t n_args;
    uint16_t n_results;
    uint64_t args[16];
    uint64_t results[1];
} CuwasmMailbox;

typedef int (*CuwasmHostFn)(void* ctx, CuwasmMailbox* mb, char* err, size_t err_cap);

typedef struct CuwasmRunResult {
    uint16_t status;
    uint64_t results[8];
    uint32_t n_results;
    char error[256];
} CuwasmRunResult;

CuwasmModule* cuwasm_module_load(const uint8_t* wasm, size_t len, char* err, size_t err_cap);
void cuwasm_module_free(CuwasmModule* m);
int cuwasm_module_export_index(CuwasmModule* m, const char* name);
uint8_t* cuwasm_module_memory(CuwasmModule* m);
uint32_t cuwasm_module_memory_size(CuwasmModule* m);
int cuwasm_module_run(CuwasmModule* m, uint32_t func_idx, const uint64_t* args, uint32_t n_args,
                      uint64_t max_steps, CuwasmHostFn host, void* ctx, CuwasmRunResult* out);

#ifdef __cplusplus
}
#endif
