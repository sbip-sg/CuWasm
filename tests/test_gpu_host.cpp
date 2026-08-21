#include "cuwasm/gpu_host_run.h"
#include "cuwasm/host.h"

#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

static int g_fail = 0, g_pass = 0;

static void expect(bool cond, const char* msg) {
    if (cond) {
        ++g_pass;
    } else {
        ++g_fail;
        std::fprintf(stderr, "FAIL: %s\n", msg);
    }
}

static bool load_wasm(const char* path, cuwasm::HostModule& m, std::string& err) {
    std::vector<uint8_t> bytes;
    if (!cuwasm::load_file(path, bytes, err)) return false;
    if (!cuwasm::translate_wasm(bytes.data(), bytes.size(), m, err)) return false;
    return cuwasm::verify_cuop(m, err);
}

int main() {
    using namespace cuwasm;
    std::string err;

    // ── increment still works with byte-wise KV ──────────────────────────
    {
        HostModule m;
        expect(load_wasm("contracts/wasm/soroban_increment_contract.wasm", m, err), "load increment");
        GpuHostState hs{};
        auto r1 = run_gpu_host(m, "increment", nullptr, 0, hs);
        expect(r1.status == ST_OK, "increment #1 ok");
        expect(r1.result == 0x100000004ULL, "increment #1 == U32(1)");
        auto r2 = run_gpu_host(m, "increment", nullptr, 0, hs);
        expect(r2.status == ST_OK, "increment #2 ok");
        expect(r2.result == 0x200000004ULL, "increment #2 == U32(2)");
    }

    // ── hello still type-checks ──────────────────────────────────────────
    {
        HostModule m;
        expect(load_wasm("contracts/wasm/soroban_hello_world_contract.wasm", m, err), "load hello");
        GpuHostState hs{};
        uint64_t arg = soroban_make_obj(0, SOROBAN_TAG_STRING_OBJECT);
        gpu_heap_reset(hs.obj_heap);
        obj_alloc(hs.obj_heap, SOROBAN_TAG_STRING_OBJECT, 5, (const uint8_t*)"World");
        auto r = run_gpu_host(m, "hello", &arg, 1, hs);
        expect(r.status == ST_OK, "hello ok");
        expect(soroban_tag(r.result) == SOROBAN_TAG_VEC_OBJECT, "hello returns VecObject");
    }

    // ── token mint → balance → transfer → balance ────────────────────────
    {
        HostModule m;
        expect(load_wasm("contracts/wasm/soroban_token_contract.wasm", m, err), "load token");

        std::vector<uint8_t> snap_mem, snap_live;
        std::vector<uint64_t> snap_glob;
        uint32_t snap_ms = 0;
        snapshot_module(m, snap_mem, snap_glob, snap_live, snap_ms);

        uint8_t alice[32], bob[32], admin[32];
        fill_pk(alice, 0xA1);
        fill_pk(bob, 0xB0);
        fill_pk(admin, 0xAD);

        GpuHostState hs{};
        gpu_seed_admin(hs.storage, admin);

        // mint(alice, 1000)
        restore_module(m, snap_mem, snap_glob, snap_live, snap_ms);
        seed_mint_args(hs, alice, 1000);
        uint64_t mint_args[2] = {
            soroban_make_obj(0, SOROBAN_TAG_ADDRESS_OBJECT),
            soroban_make_obj(1, SOROBAN_TAG_I128OBJECT),
        };
        auto rm = run_gpu_host(m, "mint", mint_args, 2, hs);
        expect(rm.status == ST_OK, "mint ok");

        // balance(alice) == 1000
        restore_module(m, snap_mem, snap_glob, snap_live, snap_ms);
        seed_balance_args(hs, alice);
        uint64_t bal_arg = soroban_make_obj(0, SOROBAN_TAG_ADDRESS_OBJECT);
        auto rb1 = run_gpu_host(m, "balance", &bal_arg, 1, hs);
        expect(rb1.status == ST_OK, "balance(alice) after mint ok");
        expect(rb1.result == soroban_i128_small(1000), "alice balance 1000");

        // transfer(alice, bob, 400)
        restore_module(m, snap_mem, snap_glob, snap_live, snap_ms);
        seed_transfer_args(hs, alice, bob, 400);
        uint64_t xfer_args[3] = {
            soroban_make_obj(0, SOROBAN_TAG_ADDRESS_OBJECT),
            soroban_make_obj(1, SOROBAN_TAG_ADDRESS_OBJECT),
            soroban_make_obj(2, SOROBAN_TAG_I128OBJECT),
        };
        auto rt = run_gpu_host(m, "transfer", xfer_args, 3, hs);
        expect(rt.status == ST_OK, "transfer ok");

        restore_module(m, snap_mem, snap_glob, snap_live, snap_ms);
        seed_balance_args(hs, alice);
        auto ra = run_gpu_host(m, "balance", &bal_arg, 1, hs);
        expect(ra.status == ST_OK, "balance(alice) after xfer ok");
        expect(ra.result == soroban_i128_small(600), "alice balance 600");

        restore_module(m, snap_mem, snap_glob, snap_live, snap_ms);
        seed_balance_args(hs, bob);
        auto rbb = run_gpu_host(m, "balance", &bal_arg, 1, hs);
        expect(rbb.status == ST_OK, "balance(bob) after xfer ok");
        expect(rbb.result == soroban_i128_small(400), "bob balance 400");

        if (rm.status != ST_OK)
            std::fprintf(stderr, "  mint status=%s err=%s\n", status_name(rm.status), rm.error.c_str());
        if (rt.status != ST_OK)
            std::fprintf(stderr, "  transfer status=%s err=%s result=0x%llx\n",
                         status_name(rt.status), rt.error.c_str(), (long long)rt.result);
        if (rb1.result != soroban_i128_small(1000))
            std::fprintf(stderr, "  alice after mint = 0x%llx\n", (long long)rb1.result);
        if (ra.result != soroban_i128_small(600))
            std::fprintf(stderr, "  alice after xfer = 0x%llx\n", (long long)ra.result);
        if (rbb.result != soroban_i128_small(400))
            std::fprintf(stderr, "  bob after xfer = 0x%llx\n", (long long)rbb.result);
    }

    std::printf("test_gpu_host: passed=%d failed=%d\n", g_pass, g_fail);
    return g_fail ? 1 : 0;
}
