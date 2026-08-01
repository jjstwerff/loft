# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The whole build for this fixture is `cc`.  No rustc, no cargo, no libffi —
# which is the property @PLN24 is for, so the build has to demonstrate it.
#
#   make          build the shared library
#   make check    build + run the self-test (validate the oracle)
#   make strict   build under -std=c89 -pedantic, proving it is ANSI C
#   make clean

CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -fPIC
UNAME_S := $(shell uname -s)

ifeq ($(UNAME_S),Darwin)
  LIB := liblc_types.dylib
  SOFLAGS := -dynamiclib
else
  LIB := liblc_types.so
  SOFLAGS := -shared
endif

.PHONY: all check strict clean

all: $(LIB)

$(LIB): lc_types.c lc_types.h
	$(CC) $(CFLAGS) $(SOFLAGS) -o $@ lc_types.c

check: lc_selftest
	./lc_selftest && echo "lc_types: oracle validated"

lc_selftest: lc_selftest.c lc_types.c lc_types.h
	$(CC) $(CFLAGS) -o $@ lc_selftest.c lc_types.c

# The claim in the header is that this is ANSI C, so it is checked rather than
# asserted.  `lc_var_sum` reads its variadic arguments as int64_t, which C89
# spells `long` on LP64 — the strict build is what keeps that honest.
strict:
	$(CC) -std=c89 -pedantic -Wall -Wextra -fPIC $(SOFLAGS) -o $(LIB) lc_types.c
	@echo "lc_types: builds as ANSI C"

clean:
	rm -f $(LIB) lc_selftest
