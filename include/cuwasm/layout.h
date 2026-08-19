#pragma once

#include "hd.h"
#include "vmstate.h"

namespace cuwasm {

struct AoSView {
    uint64_t* base;
    uint32_t cap, inst;
    HD uint64_t& at(uint32_t i) const { return base[(uint64_t)inst * cap + i]; }
};

struct AoSFrameView {
    Frame* base;
    uint32_t cap, inst;
    HD Frame& at(uint32_t i) const { return base[(uint64_t)inst * cap + i]; }
};

} // namespace cuwasm
