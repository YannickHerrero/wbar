TARGET := x86_64-pc-windows-gnu
BIN := wbar.exe
RELEASE_DIR := target/$(TARGET)/release

ifndef WIN_USER
WIN_USER := $(shell cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r\n')
endif
DEST_DIR := /mnt/c/Users/$(WIN_USER)/Documents/apps
DEST := $(DEST_DIR)/$(BIN)

.PHONY: default build install kill clean deploy
default: install

build:
	cargo build --release --target $(TARGET)

kill:
	@cmd.exe /c "taskkill /IM $(BIN) >nul 2>&1" 2>/dev/null || true
	@sleep 0.4
	@cmd.exe /c "taskkill /F /IM $(BIN) >nul 2>&1" 2>/dev/null || true

install: build kill
	@test -n "$(WIN_USER)" || { echo "ERROR: WIN_USER is empty (cmd.exe detection failed). Run with WIN_USER=<name>."; exit 1; }
	mkdir -p $(DEST_DIR)
	cp $(RELEASE_DIR)/$(BIN) $(DEST)
	@echo "Installed: $(DEST)"

clean:
	cargo clean

deploy:
	@./scripts/deploy.sh $(BUMP)
