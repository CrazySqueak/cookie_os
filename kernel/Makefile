ARCHDIR=arch/$(ARCH)
ARCHBUILDDIR=$(BUILDDIR)
include $(ARCHDIR)/make.config

RUST_MERGED_DIR=$(ARCHBUILDDIR)/rust
RUSTSRC_COMMON_DIR=rust
RUSTSRC_ARCH_DIR=$(ARCHDIR)/rust

# I spent like 45 minutes crafting this myself from google results
# before I found that statically linking with rust was literally covered 
# in the fucking tutorial
# I just hadn't read that part
# oh well, keeps my mind sharp
RUST_ARCHIVE_NAME=kernel_rs
RUST_ARCHIVE=$(RUST_MERGED_DIR)/$(RS_TARGET_DIR)/lib$(RUST_ARCHIVE_NAME).a

OBJECTS=$(ARCH_OBJS)
LINK_OBJS=$(OBJECTS) $(RUST_ARCHIVE)
LINK_LIST=$(OBJECTS) -L$(dir $(RUST_ARCHIVE)) -l$(RUST_ARCHIVE_NAME)

$(KBINNAME): $(LINK_OBJS) $(ARCHDIR)/linker.ld
	@mkdir -p $(dir $@)
	$(LD) -n -T $(ARCHDIR)/linker.ld -o $@ $(LINK_LIST)
	grub-file --is-x86-multiboot2 $@

$(ARCHBUILDDIR)/%.o: $(ARCHDIR)/%.intel.asm
	@#Assemble with NASM
	@mkdir -p $(dir $@)
	$(NASM) $^ -o $@

# Rust
rust-merge: FORCE
	@mkdir -p $(RUST_MERGED_DIR)
	if [ -d "$(RUSTSRC_COMMON_DIR)" ]; then cp -ru $(RUSTSRC_COMMON_DIR)/* $(RUST_MERGED_DIR); fi
	if [ -d "$(RUSTSRC_ARCH_DIR)" ]; then cp -ru $(RUSTSRC_ARCH_DIR)/* $(RUST_MERGED_DIR); fi

$(RUST_ARCHIVE): rust-merge
	cd $(RUST_MERGED_DIR) && cargo build $(CARGOFLAGS)

.PHONY: clean
clean:
	-rm -rf $(BUILDROOT)
	-rm -rf $(DISTROOT)

FORCE: