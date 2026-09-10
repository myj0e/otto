CC ?= cc
PKG_CONFIG ?= pkg-config
PREFIX ?= /usr/local

CPPFLAGS ?=
CPPFLAGS += -Iinclude -D_POSIX_C_SOURCE=200809L
CFLAGS ?= -std=c11 -O2 -Wall -Wextra -Wpedantic

CURL_CFLAGS := $(shell $(PKG_CONFIG) --cflags libcurl 2>/dev/null)
CURL_LIBS := $(shell $(PKG_CONFIG) --libs libcurl 2>/dev/null)
ifeq ($(strip $(CURL_LIBS)),)
CURL_LIBS := -lcurl
endif

TARGET := otto
SOURCES := $(wildcard src/*.c)

.PHONY: all check-deps test clean install

all: check-deps $(TARGET)

check-deps:
	@command -v $(CC) >/dev/null 2>&1 || \
		(echo "error: C compiler not found" >&2; exit 1)
	@command -v $(PKG_CONFIG) >/dev/null 2>&1 || \
		(echo "error: pkg-config not found" >&2; exit 1)
	@$(PKG_CONFIG) --exists libcurl || \
		(echo "error: libcurl development package not found" >&2; exit 1)

$(TARGET): $(SOURCES) $(wildcard include/otto/*.h)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(CURL_CFLAGS) -o $@ $(SOURCES) $(CURL_LIBS)

test: check-deps $(TARGET)
	sh ./tests/test.sh ./$(TARGET)
	sh ./tests/test_install.sh ./$(TARGET)

clean:
	rm -f $(TARGET) src/*.o

install: $(TARGET)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 0755 $(TARGET) $(DESTDIR)$(PREFIX)/bin/$(TARGET)
