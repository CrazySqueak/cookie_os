ARCHDIR=arch/$(ARCH)
ARCHBUILDDIR=$(BUILDDIR)
include $(ARCHDIR)/make.config

RUST_DIR=rust

# I spent like 45 minutes crafting this myself from google results
# before I found that statically linking with rust was literally covered 
# in the fucking tutorial
# I just hadn't read that part
# oh well, keeps my mind sharp
RUST_ARCHIVE_NAME=kernel_rs
RUST_ARCHIVE=$(RUST_DIR)/target/$(RS_TARGET_DIR)/lib$(RUST_ARCHIVE_NAME).a

OBJECTS=$(ARCH_OBJS)
LINK_OBJS=$(OBJECTS) $(RUST_ARCHIVE)
LINK_LIST=$(OBJECTS) -L$(dir $(RUST_ARCHIVE)) -l$(RUST_ARCHIVE_NAME)

$(KBINNAME): $(LINK_OBJS) $(ARCHDIR)/linker.ld
	@mkdir -p $(dir $@)
	$(LD) -n -T $(ARCHDIR)/linker.ld -o $@ $(LINK_LIST) --no-relax
	grub-file --is-x86-multiboot2 $@

$(ARCHBUILDDIR)/%.o: $(ARCHDIR)/%.intel.asm
	@#Assemble with NASM
	@mkdir -p $(dir $@)
	$(NASM) $^ -o $@

# Rust
$(RUST_ARCHIVE): FORCE
	cd $(RUST_DIR) && cargo build --features "$(KBUILDFEATURES)" --target=target-$(ARCH).json $(CARGOFLAGS)

.PHONY: clean
clean:
	-rm -rf $(BUILDROOT)
	-rm -rf $(DISTROOT)
	-cd $(RUST_DIR) && cargo clean

FORCE: