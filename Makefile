export ARCH ?= x86_64-elf
export LD := $(ARCH)-ld

export NASM := nasm -f elf64
export QEMU := qemu-system-x86_64

export SYSROOT := $(abspath build/sysroot)

ifeq ($(INCLUDE_DEBUG_SYMBOLS),1)
export NASM := $(NASM) -g -F dwarf
else
export LD := $(LD) -S
endif


dist/boot.iso: $(SYSROOT)/boot/kernel.bin $(SYSROOT)/boot/grub/grub.cfg
	@mkdir -p $(dir $@)
	grub-mkrescue -o $@ $(SYSROOT)

$(SYSROOT)/boot/grub/grub.cfg: grub.cfg
	@mkdir -p $(dir $@)
	cp $^ $@

$(SYSROOT)/boot/kernel.bin: FORCE
	@mkdir -p $(dir $@)
	$(MAKE) -C kernel
	cp -u kernel/dist/kernel.bin $@

clean:
	-rm -r build
	-rm -r dist
	$(MAKE) -C kernel clean

run: dist/boot.iso
	$(QEMU) --cdrom $^
debug: dist/boot.iso $(SYSROOT)/boot/kernel.bin
	$(QEMU) --cdrom $^ -s -S &
	gdb --symbols=$(SYSROOT)/boot/kernel.bin -ex "target remote localhost:1234" -ex "tui layout asm" -ex "tui layout reg"

FORCE: