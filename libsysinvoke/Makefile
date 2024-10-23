all: $(DISTROOT)/libsysinvoke.so

RUST_ARCHIVE_DIR := target/$(RS_TARGET_DIR)

$(DISTROOT)/libsysinvoke.so: $(RUST_ARCHIVE_DIR)/liblibsysinvoke.so
	@mkdir -p $(dir $@)
	cp -u $^ $@

$(RUST_ARCHIVE_DIR)/liblibsysinvoke.so &: FORCE
	cargo build --target=target-$(ARCH).json $(CARGOFLAGS)

.PHONY: clean
clean:
	cargo clean
	-rm -rf $(DISTROOT)

FORCE: