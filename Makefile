all: iso-grub iso-limine

export ARCH ?= x86_64-elf
export LD := x86_64-elf-ld
export NASM := nasm -f elf64
export QEMU := qemu-system-x86_64

export GRUB_MKRESCUE := grub-mkrescue
export LIMINE := limine

export KBUILDFEATURES ?= enable_amd64_TCE 1G_huge_pages per_page_NXE_bit page_global_bit
QEMUCPU ?= max -smp 4

export BUILDNAME := $(ARCH)

ifeq ($(INCLUDE_DEBUG_SYMBOLS),1)
$(info Including debug symbols.)
# include debug symbols
export NASM := $(NASM) -g -F dwarf
export BUILDNAME := $(BUILDNAME)-withsymbols
# include unwind tables for backtracing
export RUSTFLAGS:=-Cforce-unwind-tables $(RUSTFLAGS)
else
$(info Stripping debug symbols.)
export LD := $(LD) -S
endif

ifeq ($(RELEASE_BUILD),1)
$(info Building rust code in release mode.)
export CARGOFLAGS := $(CARGOFLAGS) --release
export RS_TARGET_DIR := target-$(ARCH)/release
else
$(info Building rust code in development mode.)
export RS_TARGET_DIR := target-$(ARCH)/debug
export BUILDNAME := $(BUILDNAME)-rsdev
endif

export BUILDROOT := build
export BUILDDIR := $(BUILDROOT)/$(BUILDNAME)
export DISTROOT := dist
export KBINNAME := $(DISTROOT)/kernel-$(BUILDNAME).bin

SYSROOT := $(abspath $(BUILDDIR)/sysroot)

## Sub-modules
# kernel
KERNEL_BIN := kernel/$(KBINNAME)
$(KERNEL_BIN): FORCE
	$(MAKE) -C kernel

# QEMU config
export QLOGSDIR := logs
QLOGNAME := $(QLOGSDIR)/$(shell date +"%Y-%m-%dT%Hh%Mm%S")-$(BUILDNAME)-serial.log
# Note: tee is WAY faster than a chardev here. presumably tee uses buffered io which is faster (especially since win<->wsl is relatively slow and high-latency)
QLOGARGSRUN := -serial stdio
QLOGARGSDBG := -serial file:$(QLOGNAME)

## GRUB
# GRUB ISO file (CD)
GISOROOT := $(abspath $(BUILDDIR)/grubiso)
GISONAME := $(DISTROOT)/boot-$(BUILDNAME)-grub.iso

$(GISONAME): $(GISOROOT)/boot/kernel.bin $(GISOROOT)/boot/grub/grub.cfg
	@mkdir -p $(dir $@)
	grub-mkrescue -o $@ $(GISOROOT)

$(GISOROOT)/boot/grub/grub.cfg: grub.cfg
	@mkdir -p $(dir $@)
	cp -u $^ $@

$(GISOROOT)/boot/kernel.bin: $(KERNEL_BIN)
	@mkdir -p $(dir $@)
	cp -u $(KERNEL_BIN) $@

## LIMINE
# Limine ISO file (CD)
LISOROOT := $(abspath $(BUILDDIR)/limineiso)
LISONAME := $(DISTROOT)/boot-$(BUILDNAME)-limine.iso

LIMINEDATADIR := $(shell $(LIMINE) --print-datadir)
LIMINE_BIOS_FILES := $(addprefix $(LISOROOT)/boot/limine/,limine-bios.sys limine-bios-cd.bin limine-uefi-cd.bin)
LIMINE_UEFI_FILES := $(addprefix $(LISOROOT)/EFI/BOOT/,BOOTX64.EFI BOOTIA32.EFI)

$(LISONAME): $(LISOROOT)/boot/kernel.bin $(LISOROOT)/boot/limine/limine.conf $(LIMINE_BIOS_FILES) $(LIMINE_UEFI_FILES)
	@mkdir -p $(dir $@)
	@# Make the ISO
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table \
	--efi-boot boot/limine/limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label \
	$(LISOROOT) -o $@
	@# Install legacy BIOS boot
	$(LIMINE) bios-install $@

$(LISOROOT)/boot/limine/limine.conf: limine.conf
	@mkdir -p $(dir $@)
	cp -u $^ $@

$(LISOROOT)/boot/kernel.bin: $(KERNEL_BIN)
	@mkdir -p $(dir $@)
	cp -u $(KERNEL_BIN) $@

$(LIMINE_BIOS_FILES): $(LISOROOT)/boot/limine/%: $(LIMINEDATADIR)/%
	@mkdir -p $(dir $@)
	cp -u $^ $@

$(LIMINE_UEFI_FILES): $(LISOROOT)/EFI/BOOT/%: $(LIMINEDATADIR)/%
	@mkdir -p $(dir $@)
	cp -u $^ $@

## Targets
# clean targets
clean:
	-rm -r $(BUILDROOT)
	-rm -r $(DISTROOT)
	-rm -r $(QLOGSDIR)

clean-all: clean
	$(MAKE) -C kernel clean

# build targets 
iso-grub: $(GISONAME)
# TODO
iso-limine: $(LISONAME)

# run targets
QEMU_RUN_MODE ?= grub-cd

QEMURM-deps-grub-cd := iso-grub
QEMURM-args-grub-cd := --cdrom $(GISONAME)

QEMURM-deps-limine-cd := iso-limine
QEMURM-args-limine-cd := --cdrom $(LISONAME)

QEMUTARGETDEPS := $(QEMURM-deps-$(QEMU_RUN_MODE))
QEMUTARGETARGS := $(QEMURM-args-$(QEMU_RUN_MODE))

check-qemu-var:
	@if [ -z "$(QEMUTARGETDEPS)" ]; then \
		echo "ERROR: QEMU_RUN_MODE was set to an invalid value: $$QEMU_RUN_MODE";\
		echo "Valid Modes: grub-cd limine-cd";\
		echo "Or unset it to use the default value (grub-cd).";\
		exit 1;\
	fi

run: check-qemu-var $(QEMUTARGETDEPS)
	@mkdir -p $(dir $(QLOGNAME))
	$(QEMU) $(QEMUTARGETARGS) -cpu $(QEMUCPU) $(QLOGARGSRUN) $(QEMUARGS) | tee $(QLOGNAME)
debug: check-qemu-var $(QEMUTARGETDEPS) $(KERNEL_BIN)
	@if [ "$$INCLUDE_DEBUG_SYMBOLS" != "1" ]; then\
		echo -e "\033[0;33mWARNING: Debug symbols were not included in this build! Set $$INCLUDE_DEBUG_SYMBOLS to 1 to include them!\033[0m";\
		sleep 1;\
	fi
	$(QEMU) $(QEMUTARGETARGS) -cpu $(QEMUCPU) $(QLOGARGSDBG) $(QEMUARGS) -s -S >/dev/null &
	@echo "Serial log can be found at: $(QLOGNAME)"
	gdb -q --symbols=$(KERNEL_BIN) -ex "target remote localhost:1234"
# Check that everything compiles for the qemu target, but don't actually launch qemu even on success
check: check-qemu-var $(QEMUTARGETDEPS)

# Check that everything compiles correctly, but doesn't build the final ISO
compile: $(KERNEL_BIN)

# special targets
FORCE:

.PHONY: all clean clean-all iso-grub iso-limine run debug check compile check-qemu-var