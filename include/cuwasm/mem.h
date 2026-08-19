#pragma once

#include "hd.h"

#include <cstdint>

namespace cuwasm {

static constexpr uint32_t WASM_PAGE = 65536;
static constexpr uint32_t CUWASM_MEM_MAX_PAGES = 1024;
static constexpr uint32_t TABLE_NULL = 0xFFFFFFFFu;

struct MemView {
    uint8_t* data;
    uint32_t size;
    uint32_t max_size;
};

struct DataView {
    uint8_t* blob;
    uint32_t blob_len;
    const uint32_t* off;
    const uint32_t* len;
    uint8_t* live;
    uint32_t n;
};

struct TableView {
    uint32_t* elems;
    uint32_t size;
};

HD bool mem_in_bounds(const MemView& m, uint64_t ea, uint32_t n) {
    if (!m.data && n != 0)
        return false;
    if ((uint64_t)n > (uint64_t)m.size)
        return false;
    return ea <= (uint64_t)m.size - (uint64_t)n;
}

HD uint64_t mem_load_le(const MemView& m, uint64_t ea, uint32_t n) {
    uint64_t v = 0;
    for (uint32_t i = 0; i < n; ++i)
        v |= (uint64_t)m.data[ea + i] << (8 * i);
    return v;
}

HD void mem_store_le(MemView& m, uint64_t ea, uint32_t n, uint64_t v) {
    for (uint32_t i = 0; i < n; ++i)
        m.data[ea + i] = (uint8_t)(v >> (8 * i));
}

HD uint64_t sext64(uint64_t v, uint32_t bits) {
    uint32_t sh = 64 - bits;
    return (uint64_t)((int64_t)(v << sh) >> sh);
}

} // namespace cuwasm
