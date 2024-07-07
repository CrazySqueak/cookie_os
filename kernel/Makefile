ARCHDIR=arch/$(ARCH)
ARCHBUILDDIR=$(BUILDDIR)
include $(ARCHDIR)/make.config

OBJECTS=$(ARCH_OBJS)
LINK_OBJS=$(OBJECTS)
LINK_LIST=$(OBJECTS)

$(KBINNAME): $(LINK_OBJS) $(ARCHDIR)/linker.ld
	@mkdir -p $(dir $@)
	$(LD) -n -T $(ARCHDIR)/linker.ld -o $@ $(LINK_LIST)
	grub-file --is-x86-multiboot2 $@

$(ARCHBUILDDIR)/%.o: $(ARCHDIR)/%.intel.asm
	@#Assemble with NASM
	@mkdir -p $(dir $@)
	$(NASM) $^ -o $@

.PHONY: clean
clean:
	-rm -rf $(BUILDROOT)
	-rm -rf $(DISTROOT)