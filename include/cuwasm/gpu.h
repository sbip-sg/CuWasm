#pragma once

#include "host.h"

namespace cuwasm {

RunResult run_gpu(HostModule& m, uint32_t func_idx, const uint64_t* args,
                  uint32_t n_args, uint64_t max_steps = DEFAULT_MAX_STEPS,
                  HostFn host_fn = default_host_fn);

} // namespace cuwasm
