ifneq ($(KERNELRELEASE),)
# kbuild part of makefile
include Kbuild

else
# normal makefile
KDIR ?= /lib/modules/`uname -r`/build
# KDIR ?= /home/vagrant/kern_src/linux-6.2/

.PHONY: all
all:
	$(MAKE) -C $(KDIR) M=$$PWD modules

.PHONY: install
install: all install_modules
	sudo depmod -A && \
    for file in *.ko; do \
		cc_name="$$(basename "$$file" .ko)"; \
		sudo modprobe "$$cc_name"; \
		sudo sysctl -w net.ipv4.tcp_congestion_control="$$cc_name"; \
	done

.PHONY: install_modules
install_modules: all
	$(MAKE) -C $(KDIR) M=$$PWD modules_install
	sudo depmod -A

.PHONY: helpl
help:
	$(MAKE) -C $(KDIR) M=$$PWD help

.PHONY: clean
clean:
	$(MAKE) -C $(KDIR) M=$$PWD clean
# Module specific targets
# genbin:
#         echo "X" > 8123_bin.o_shipped

endif

# all:
# 		make -C /lib/modules/$(shell uname -r)/build M=$(PWD) modules

# clean:
# 		make -C /lib/modules/$(shell uname -r)/build M=$(PWD) clean
