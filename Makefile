PREFIX ?= $(HOME)/.local
BINDIR = $(PREFIX)/bin
APPDIR = $(PREFIX)/share/applications
ICONDIR = $(PREFIX)/share/icons/hicolor
DESKTOP = io.github.ileskaa.PdfReader.desktop

.PHONY: build install uninstall

build:
	cargo build --release --locked

install: build
	install -Dm755 target/release/pdf-reader $(BINDIR)/pdf-reader
	install -Dm644 data/$(DESKTOP) $(APPDIR)/$(DESKTOP)
	install -Dm644 data/icons/pdf-reader.svg $(ICONDIR)/scalable/apps/pdf-reader.svg
	-update-desktop-database $(APPDIR)
	-gtk4-update-icon-cache -f -t $(ICONDIR)

uninstall:
	rm -f $(BINDIR)/pdf-reader $(APPDIR)/$(DESKTOP) $(ICONDIR)/scalable/apps/pdf-reader.svg
	-update-desktop-database $(APPDIR)
	-gtk4-update-icon-cache -f -t $(ICONDIR)
