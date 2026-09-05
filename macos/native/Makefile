# IOKit unlock experiments for docs/MACOS-NATIVE.md §4.1.
#
# These are throwaway probes, deliberately kept OUT of the Rust crate: a negative result should
# cost nothing, and iterating on IOKit is faster in C. Only whichever one passes gets ported.
#
#   make            build both
#   make e1         E1/E2 — IOHIDManager SetReport (+ optional seize)
#   make e3         E3    — USBInterfaceOpenSeize + WritePipe
#   make run-e1     build and dry-run E1
#
# Requires the Xcode Command Line Tools (`xcode-select --install`). macOS only.

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O1 -g
FRAMEWORKS = -framework CoreFoundation -framework IOKit

BINS = e1_hid_setreport e3_interface_seize

.PHONY: all e1 e3 run-e1 run-e3 clean
all: $(BINS)

e1: e1_hid_setreport
e3: e3_interface_seize

e1_hid_setreport: e1_hid_setreport.c
	$(CC) $(CFLAGS) -o $@ $< $(FRAMEWORKS)

e3_interface_seize: e3_interface_seize.c
	$(CC) $(CFLAGS) -o $@ $< $(FRAMEWORKS)

# Dry runs send nothing — they enumerate and describe what they found.
run-e1: e1_hid_setreport
	./e1_hid_setreport

run-e3: e3_interface_seize
	sudo ./e3_interface_seize

clean:
	rm -f $(BINS)
	rm -rf $(BINS:=.dSYM)
