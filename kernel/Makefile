ARCHDIR=arch/$(ARCH)
ARCHBUILDDIR=build/$(ARCH)
include $(ARCHDIR)/make.config

OBJECTS=$(ARCH_OBJS)
LINK_OBJS=$(OBJECTS)
LINK_LIST=$(OBJECTS)

dist/kernel.bin: $(LINK_OBJS) $(ARCHDIR)/linker.ld
	@mkdir -p $(dir $@)
	$(LD) -n -T $(ARCHDIR)/linker.ld -o $@ $(LINK_LIST)
	grub-file --is-x86-multiboot2 $@

$(ARCHBUILDDIR)/%.o: $(ARCHDIR)/%.intel.asm
	@#Assemble with NASM
	@mkdir -p $(dir $@)
	$(NASM) $^ -o $@

.PHONY: clean
clean:
	-rm -rf build
	-rm -rf dist