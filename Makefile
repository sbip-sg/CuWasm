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

CPU_SRCS := src/translate.cpp src/verify.cpp src/disasm.cpp src/run.cpp
TEST_SRCS := tests/test_main.cpp $(CPU_SRCS)

.PHONY: all verify test-cpu test-gpu prep tools clean

all: $(BUILD)/cuwasm-run $(BUILD)/test_cpu

$(RUSTLIB): tools/src/lib.rs tools/src/bin/oracle.rs tools/src/bin/wastprep.rs tools/Cargo.toml
	mkdir -p $(BUILD)
	$(CARGO_TIMEOUT) env CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build --release --manifest-path $(TOOLS_MANIFEST)

$(ORACLE): $(RUSTLIB)
	mkdir -p $(BUILD)
	cp -f $(CARGO_TARGET_DIR)/release/cuwasm-oracle $(ORACLE)
	cp -f $(CARGO_TARGET_DIR)/release/wastprep $(WASTPREP)

$(WASTPREP): $(ORACLE)

prep: $(ORACLE)
	mkdir -p $(GEN)
	$(TIMEOUT) $(WASTPREP) tests/fibonacci.wast $(GEN)

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

clean:
	rm -rf $(BUILD)
