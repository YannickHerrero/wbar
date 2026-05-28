UNAME_S := $(shell uname -s)

ifeq ($(UNAME_S),Darwin)
# Native build on the host arch; arm64 -> aarch64 to match Rust's triple.
TARGET := $(shell uname -m | sed 's/arm64/aarch64/')-apple-darwin
BIN := wbar
DEST_DIR := $(HOME)/.local/bin
KILL_CMD := pkill -x wbar 2>/dev/null || true
KILL_FORCE_CMD := pkill -9 -x wbar 2>/dev/null || true
DEST_GUARD := true
else
TARGET := x86_64-pc-windows-gnu
BIN := wbar.exe
ifndef WIN_USER
WIN_USER := $(shell cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r\n')
endif
DEST_DIR := /mnt/c/Users/$(WIN_USER)/Documents/apps
KILL_CMD := cmd.exe /c "taskkill /IM $(BIN) >nul 2>&1" 2>/dev/null || true
KILL_FORCE_CMD := cmd.exe /c "taskkill /F /IM $(BIN) >nul 2>&1" 2>/dev/null || true
DEST_GUARD := test -n "$(WIN_USER)" || { echo "ERROR: WIN_USER is empty (cmd.exe detection failed). Run with WIN_USER=<name>."; exit 1; }
endif

RELEASE_DIR := target/$(TARGET)/release
DEST := $(DEST_DIR)/$(BIN)

.PHONY: default build install kill clean deploy
default: install

build:
	cargo build --release --target $(TARGET)

kill:
	@$(KILL_CMD)
	@sleep 0.4
	@$(KILL_FORCE_CMD)

install: build kill
	@$(DEST_GUARD)
	mkdir -p $(DEST_DIR)
	cp $(RELEASE_DIR)/$(BIN) $(DEST)
	@echo "Installed: $(DEST)"

clean:
	cargo clean

deploy:
	@./scripts/deploy.sh $(BUMP)
