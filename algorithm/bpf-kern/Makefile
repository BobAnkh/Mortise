OUTPUT := build
CLANG ?= clang
LLVM_STRIP ?= llvm-strip
INCLUDES := -I$(OUTPUT) -Iinclude
CFLAGS := -g -Wall
SKEL_OUTPUT := skeleton
BPFTOOL := bpftool
LLC := llc

SRCS := $(wildcard *.bpf.c)
BPF_OBJS := $(patsubst %.bpf.c,$(OUTPUT)/%.bpf.o,$(SRCS))
SKEL_OBJS := $(patsubst %.bpf.c,$(SKEL_OUTPUT)/%.skel.h,$(SRCS))

# Emit succinct information message describing current building step
# $1 - generic step name (e.g., CC, LINK, etc);
# $2 - optional "flavor" specifier; if provided, will be emitted as [flavor];
# $3 - target (assumed to be file); only file name will be emitted;
# $4 - optional extra arg, emitted as-is, if provided.
ifeq ($(V),1)
	Q =
else
	Q = @
	MAKEFLAGS += --no-print-directory
	submake_extras := feature_display=0
endif
msg = @printf '  %-8s%s %s%s\n' "$(1)" "$(if $(2), [$(2)])" "$(notdir $(3))" "$(if $(4), $(4))";

.PHONY: all
all: $(BPF_OBJS)

.PHONY: skel
skel: $(SKEL_OBJS)

.PHONY: vmlinux
vmlinux: include/vmlinux.h

.PHONY: clean
clean:
	$(call msg,CLEAN,,$(OUTPUT))
	$(Q)rm -rf $(OUTPUT)
	$(call msg,CLEAN,,$(SKEL_OUTPUT))
	$(Q)rm -rf $(SKEL_OUTPUT)

$(OUTPUT):
	$(call msg,MKDIR,,$@)
	$(Q)mkdir -p $@

$(SKEL_OUTPUT):
	$(call msg,MKDIR,,$@)
	$(Q)mkdir -p $@

$(OUTPUT)/%.bpf.o: $(OUTPUT)/%.bpf.bc
	$(call msg,CLNG-BPF,OBJ,$@)
	$(Q)$(LLC) -march=bpf -mcpu=v3 -mattr=+alu32 -filetype=obj -o $@ $<
	$(Q)$(LLVM_STRIP) -g $@

$(OUTPUT)/%.bpf.bc: %.bpf.c include/vmlinux.h $(wildcard %.h) | $(OUTPUT)
	$(call msg,CLNG-BPF,BC,$@)
	$(Q)$(CLANG) $(CFLAGS) -g -O2 -target bpf -emit-llvm $(INCLUDES) -c $(filter %.c,$^) -o $@

$(OUTPUT)/%.bpf.ll: %.bpf.c include/vmlinux.h $(wildcard %.h) | $(OUTPUT)
	$(call msg,CLNG-IR,,$@)
	$(Q)$(CLANG) $(CFLAGS) -S -emit-llvm -g -O2 -target bpf $(INCLUDES) $(filter %.c,$^) -o $@

$(SKEL_OUTPUT)/%.skel.h: $(OUTPUT)/%.bpf.o | $(SKEL_OUTPUT)
	$(call msg,GEN-SKEL,,$@)
	$(Q)$(BPFTOOL) gen skeleton $< > $@

include/vmlinux.h:
	$(call msg,GEN-VMLIN,,$@)
	$(Q)$(BPFTOOL) btf dump file /sys/kernel/btf/vmlinux format c > $@
