export ARCH ?= x86_64-elf
export LD := x86_64-elf-ld
export NASM := nasm -f elf64
export QEMU := qemu-system-x86_64

export KBUILDFEATURES ?= per_page_NXE_bit enable_amd64_TCE page_global_bit
export QEMUCPU ?= qemu64,+pdpe1gb,+smep,+tce

export BUILDNAME := $(ARCH)

ifeq ($(INCLUDE_DEBUG_SYMBOLS),1)
$(info Including debug symbols.)
export NASM := $(NASM) -g -F dwarf
export BUILDNAME := $(BUILDNAME)-withsymbols
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
export SYSROOT := $(abspath $(BUILDDIR)/sysroot)
export DISTROOT := dist
export ISONAME := $(DISTROOT)/boot-$(BUILDNAME).iso
export KBINNAME := $(DISTROOT)/kernel-$(BUILDNAME).bin

export QLOGSDIR := logs
QLOGNAME := $(QLOGSDIR)/$(shell date +"%Y-%m-%dT%Hh%Mm%S")-$(BUILDNAME)-serial.log
# Note: tee is WAY faster than a chardev here. presumably tee uses buffered io which is faster (especially since win<->wsl is relatively slow and high-latency)
QLOGARGSRUN := -serial stdio
QLOGARGSDBG := -serial file:$(QLOGNAME)

$(ISONAME): $(SYSROOT)/boot/kernel.bin $(SYSROOT)/boot/grub/grub.cfg
	@mkdir -p $(dir $@)
	grub-mkrescue -o $@ $(SYSROOT)

$(SYSROOT)/boot/grub/grub.cfg: grub.cfg
	@mkdir -p $(dir $@)
	cp $^ $@

$(SYSROOT)/boot/kernel.bin: FORCE
	@mkdir -p $(dir $@)
	$(MAKE) -C kernel
	cp -u kernel/$(KBINNAME) $@

clean:
	-rm -r $(BUILDROOT)
	-rm -r $(DISTROOT)
	-rm -r $(QLOGSDIR)
	$(MAKE) -C kernel clean

run: $(ISONAME)
	@mkdir -p $(dir $(QLOGNAME))
	$(QEMU) --cdrom $(ISONAME) -cpu $(QEMUCPU) $(QLOGARGSRUN) $(QEMUARGS) | tee $(QLOGNAME)
debug: $(ISONAME) $(SYSROOT)/boot/kernel.bin
	@if [ "$$INCLUDE_DEBUG_SYMBOLS" != "1" ]; then\
		echo -e "\033[0;33mWARNING: Debug symbols were not included in this build! Set $$INCLUDE_DEBUG_SYMBOLS to 1 to include them!\033[0m";\
		sleep 1;\
	fi
	$(QEMU) --cdrom $(ISONAME) -cpu $(QEMUCPU) $(QLOGARGSDBG) $(QEMUARGS) -s -S >/dev/null &
	@echo "Serial log can be found at: $(QLOGNAME)"
	gdb -q --symbols=$(SYSROOT)/boot/kernel.bin -ex "target remote localhost:1234"

FORCE: