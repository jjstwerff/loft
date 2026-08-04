# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The whole build for this fixture is `cc`.  No rustc, no cargo, no libffi —
# which is the property @PLN24 is for, so the build has to demonstrate it.
#
#   make          build the shared library
#   make check    build + run the self-test (validate the oracle)
#   make probe    build + run the @PLN24 trampoline probe against it
#   make strict   build under -std=c89 -pedantic, proving it is ANSI C
#   make clean

CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -fPIC
RUSTC ?= rustc
UNAME_S := $(shell uname -s)

ifeq ($(UNAME_S),Darwin)
  LIB := liblc_types.dylib
  # `-install_name @rpath/…` is load-bearing, not decoration.  macOS records an
  # install name INSIDE the library, and every binary that links it copies that
  # string in as the thing to ask `dyld` for.  Built plain, the name is the bare
  # `liblc_types.dylib`, which `dyld` looks for on the system paths only — the
  # `-Wl,-rpath` loft emits for a library shipped beside its package is not
  # consulted, because a bare name is not rpath-relative.  The link then
  # succeeds and the program dies at startup:
  #
  #   dyld: Library not loaded: liblc_types.dylib
  #     tried: 'liblc_types.dylib' (no such file), …
  #
  # This is what a library MEANT to ship beside a package has to declare.
  SOFLAGS := -dynamiclib -Wl,-install_name,@rpath/liblc_types.dylib
else ifneq (,$(findstring NT,$(UNAME_S)))
  # Windows (MSYS/MinGW `uname -s` is MINGW64_NT-… / MSYS_NT-…).  A DLL is not
  # linkable on its own: MSVC `link.exe` links the IMPORT LIBRARY that describes
  # it, and `cc -shared` writes one only when asked.  Without this the Windows
  # leg died at
  #
  #   LINK : fatal error LNK1181: cannot open input file 'lc_types.lib'
  #
  # The `.lib` name is not free to choose — loft passes `-l lc_types` for this
  # package, having taken the stem from the declared `liblc_types.so`, so the
  # import library has to be exactly that stem.  The DLL keeps the `lib` prefix
  # because `platform::lib_variants` tries `liblc_types.dll` alongside
  # `lc_types.dll`, and a MinGW-built library conventionally keeps it.
  LIB := liblc_types.dll
  SOFLAGS := -shared -Wl,--out-implib,lc_types.lib
else
  LIB := liblc_types.so
  SOFLAGS := -shared
endif

.PHONY: all check probe strict clean

all: $(LIB)

$(LIB): lc_types.c lc_types.h
	$(CC) $(CFLAGS) $(SOFLAGS) -o $@ lc_types.c

check: lc_selftest
	./lc_selftest && echo "lc_types: oracle validated"

lc_selftest: lc_selftest.c lc_types.c lc_types.h
	$(CC) $(CFLAGS) -o $@ lc_selftest.c lc_types.c

# The @PLN24 design probe.  A single rustc invocation — no cargo, no crates —
# because the question is whether loft-core (Rust) can express the per-arity
# trampolines soundly, and a probe that needed a dependency to answer it would
# be answering a different question.
probe: probe_trampoline $(LIB)
	./probe_trampoline

probe_trampoline: probe_trampoline.rs
	$(RUSTC) --edition 2024 -O -o $@ probe_trampoline.rs

# The claim in the header is that this is ANSI C, so it is checked rather than
# asserted.  `lc_var_sum` reads its variadic arguments as int64_t, which C89
# spells `long` on LP64 — the strict build is what keeps that honest.
strict:
	$(CC) -std=c89 -pedantic -Wall -Wextra -fPIC $(SOFLAGS) -o $(LIB) lc_types.c
	@echo "lc_types: builds as ANSI C"

clean:
	rm -f $(LIB) lc_types.lib lc_selftest probe_trampoline
