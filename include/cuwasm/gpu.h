#pragma once

#include "host.h"

namespace cuwasm {

RunResult run_gpu(const HostModule& m, uint32_t func_idx, const uint64_t* args,
                  uint32_t n_args, uint64_t max_steps = DEFAULT_MAX_STEPS);

} // namespace cuwasm
