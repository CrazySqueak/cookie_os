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
# Weird cargo bug or something?? libkernel_rs.a is placed in target/target/<profile>/ instead of target/<profile>
RUST_ARCHIVE=$(RUST_MERGED_DIR)/target/$(RS_TARGET_DIR)/lib$(RUST_ARCHIVE_NAME).a

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
	@# Merge source files
	if [ -d "$(RUSTSRC_COMMON_DIR)" ]; then cp -Tru $(RUSTSRC_COMMON_DIR) $(RUST_MERGED_DIR); fi
	if [ -d "$(RUSTSRC_ARCH_DIR)" ]; then cp -Tru $(RUSTSRC_ARCH_DIR) $(RUST_MERGED_DIR); fi
	@# Merge cargo.toml (keeping mtime correct)
	if [ -f "$(RUSTSRC_ARCH_DIR)/Cargo.toml.merge" ]; then \
	toml-union "$(RUSTSRC_COMMON_DIR)/Cargo.toml" "$(RUSTSRC_ARCH_DIR)/Cargo.toml.merge" -o "$(RUST_MERGED_DIR)/Cargo.toml";\
	python3 -c 'import os;os.utime("$(RUST_MERGED_DIR)/Cargo.toml",tuple([int(max(fn("$(RUSTSRC_COMMON_DIR)/Cargo.toml"),fn("$(RUSTSRC_ARCH_DIR)/Cargo.toml.merge"))) for fn in [os.path.getatime,os.path.getmtime]]))';\
	fi

$(RUST_ARCHIVE): rust-merge
	cd $(RUST_MERGED_DIR) && cargo build $(CARGOFLAGS)

.PHONY: clean
clean:
	-rm -rf $(BUILDROOT)
	-rm -rf $(DISTROOT)

FORCE: