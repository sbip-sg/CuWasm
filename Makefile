CXX ?= g++
NVCC ?= nvcc
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Iinclude
NVCCFLAGS ?= -std=c++17 -O2 -Iinclude --compiler-options -Wall
TIMEOUT ?= timeout 60
CARGO_TIMEOUT ?= timeout 180

BUILD := build
GEN := $(BUILD)/gen
TOOLS_MANIFEST := tools/Cargo.toml
CARGO_TARGET_DIR := $(BUILD)/rust
RUSTLIB := $(CARGO_TARGET_DIR)/release/libcuwasm_translate.a
ORACLE := $(BUILD)/cuwasm-oracle
WASTPREP := $(BUILD)/wastprep
RUST_LIBS := -ldl -lpthread -lm -lgcc_s

.PHONY: test-hello-world test-increment test-token test-contract-tests emit-profiles
test-hello-world: $(RUSTLIB)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/contract-tests cargo test --release --manifest-path tools/contract-tests/Cargo.toml test_hello_world -- --nocapture

test-increment: $(RUSTLIB)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/contract-tests cargo test --release --manifest-path tools/contract-tests/Cargo.toml test_increment -- --nocapture

test-token: $(RUSTLIB)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/contract-tests cargo test --release --manifest-path tools/contract-tests/Cargo.toml test_token -- --nocapture

test-contract-tests: $(RUSTLIB)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/contract-tests cargo test --release --manifest-path tools/contract-tests/Cargo.toml -- --nocapture

emit-profiles: $(RUSTLIB)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/contract-tests cargo run --release --manifest-path tools/contract-tests/Cargo.toml --bin emit-profiles

CPU_SRCS := src/translate.cpp src/verify.cpp src/disasm.cpp src/run.cpp src/capi.cpp
TEST_SRCS := tests/test_main.cpp $(CPU_SRCS)

.PHONY: all verify test-cpu test-gpu prep tools clean suite test-host-spike spec-suite spec-catalog

all: $(BUILD)/cuwasm-run $(BUILD)/test_cpu

$(RUSTLIB): tools/src/lib.rs tools/src/env_fn_id.rs tools/src/bin/oracle.rs tools/src/bin/wastprep.rs tools/src/bin/wast-catalog.rs tools/src/bin/wat2wasm.rs tools/Cargo.toml
	mkdir -p $(BUILD)
	$(CARGO_TIMEOUT) env CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build --release --manifest-path $(TOOLS_MANIFEST)

$(ORACLE): $(RUSTLIB)
	mkdir -p $(BUILD)
	cp -f $(CARGO_TARGET_DIR)/release/cuwasm-oracle $(ORACLE)
	cp -f $(CARGO_TARGET_DIR)/release/wastprep $(WASTPREP)

$(WASTPREP): $(ORACLE)

$(BUILD)/wast-catalog: $(RUSTLIB)
	mkdir -p $(BUILD)
	cp -f $(CARGO_TARGET_DIR)/release/wast-catalog $(BUILD)/wast-catalog

SUITE_ROOT ?= tests/wasmi-tests
SUITE_OUT := $(BUILD)/suite

$(SUITE_OUT)/catalog.jsonl: $(BUILD)/wast-catalog
	mkdir -p $(SUITE_OUT)
	$(TIMEOUT) $(BUILD)/wast-catalog $(SUITE_ROOT) $(SUITE_OUT) tests/spec

$(BUILD)/test_suite: tests/test_suite.cpp $(CPU_SRCS) $(RUSTLIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -o $@ tests/test_suite.cpp $(CPU_SRCS) $(RUSTLIB) $(RUST_LIBS)

.PHONY: suite
suite: $(SUITE_OUT)/catalog.jsonl $(BUILD)/test_suite
	$(TIMEOUT) $(BUILD)/test_suite $(SUITE_OUT)/catalog.jsonl $(SUITE_OUT)

SPEC_ROOT := tests/spec
SPEC_OUT := $(BUILD)/spec-suite

$(SPEC_OUT)/catalog.jsonl: $(BUILD)/wast-catalog $(wildcard tests/spec/*.wast)
	mkdir -p $(SPEC_OUT)
	$(TIMEOUT) $(BUILD)/wast-catalog $(SPEC_ROOT) $(SPEC_OUT)

.PHONY: spec-catalog spec-suite
spec-catalog: $(SPEC_OUT)/catalog.jsonl

spec-suite: $(SPEC_OUT)/catalog.jsonl $(BUILD)/test_suite
	$(TIMEOUT) $(BUILD)/test_suite $(SPEC_OUT)/catalog.jsonl $(SPEC_OUT)

prep: $(ORACLE) build/import_host.wasm
	mkdir -p $(GEN)
	$(TIMEOUT) $(WASTPREP) tests/fibonacci.wast $(GEN)

build/import_host.wasm: tests/import_host.wat $(RUSTLIB)
	mkdir -p build
	$(CARGO_TIMEOUT) env CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo run --release --manifest-path $(TOOLS_MANIFEST) --bin wat2wasm tests/import_host.wat build/import_host.wasm

$(BUILD)/cuwasm-run: src/main_cpu.cpp $(CPU_SRCS) $(RUSTLIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -o $@ src/main_cpu.cpp $(CPU_SRCS) $(RUSTLIB) $(RUST_LIBS)

$(BUILD)/test_cpu: $(TEST_SRCS) $(RUSTLIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -o $@ $(TEST_SRCS) $(RUSTLIB) $(RUST_LIBS)

$(BUILD)/cuwasm-run-gpu: src/runner.cu $(CPU_SRCS) $(RUSTLIB)
	mkdir -p $(BUILD)
	$(NVCC) $(NVCCFLAGS) -DCUWASM_GPU_MAIN -o $@ src/runner.cu $(CPU_SRCS) $(RUSTLIB) $(RUST_LIBS)

$(BUILD)/test_gpu: tests/test_main.cpp src/runner.cu $(CPU_SRCS) $(RUSTLIB)
	mkdir -p $(BUILD)
	$(NVCC) $(NVCCFLAGS) -DCUWASM_TEST_GPU -o $@ tests/test_main.cpp src/runner.cu $(CPU_SRCS) $(RUSTLIB) $(RUST_LIBS)

test-cpu: prep $(BUILD)/test_cpu $(BUILD)/cuwasm-run
	$(TIMEOUT) $(BUILD)/test_cpu --cpu --wast tests/fibonacci.wast --gen $(GEN) --oracle $(ORACLE)

test-gpu: prep $(BUILD)/test_gpu
	$(TIMEOUT) $(BUILD)/test_gpu --t8 --wast tests/fibonacci.wast --gen $(GEN)

verify: test-cpu test-gpu
	@echo "verify ok"

HOST_SPIKE := tools/host-spike/Cargo.toml
test-host-spike:
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/host-spike cargo test --manifest-path $(HOST_SPIKE)
	$(TIMEOUT) env CARGO_TARGET_DIR=$(BUILD)/host-spike cargo run --release --manifest-path $(HOST_SPIKE)

clean:
	rm -rf $(BUILD)
