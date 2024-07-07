export ARCH ?= x86_64-elf
export LD := $(ARCH)-ld

export NASM := nasm -f elf64

export SYSROOT := $(abspath build/sysroot)

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

FORCE: