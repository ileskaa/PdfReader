A lightweight GTK4/libadwaita PDF reader for Linux, built on MuPDF.

The reader focuses on two details that Linux PDF viewers often get wrong:

- **Crisp text.** MuPDF rasterises each page at the exact device-pixel size reserved by the
  layout, so pages are displayed 1:1 without resampling. Zoom changes trigger re-rasterisation
  instead of stretching an old bitmap, and opaque rendering keeps glyph anti-aliasing tied to
  the page colour.
- **Browser-quality text selection.** Drag to select characters, double-click words, and
  triple-click lines; dragging after a double-click extends the selection by whole words.
  Selection spans pages, autoscrolls at the viewport edge, renders as continuous bands, and
  copies to both `CLIPBOARD` and `PRIMARY`.

It also provides continuous vertical scrolling, fit-width and fit-page zoom, find-in-page, an
outline sidebar, and internal and external links.

## Installing

### Build prerequisites

MuPDF is vendored and compiled by the `mupdf` crate, so no MuPDF system package is needed.
You do need a C/C++ toolchain, `clang` (for bindgen), `fontconfig`, GTK 4 and libadwaita.

Arch Linux:

```sh
sudo pacman -S --needed rustup clang gtk4 libadwaita fontconfig pkgconf base-devel
rustup default stable
```

Debian / Ubuntu:

```sh
sudo apt install rustup clang libgtk-4-dev libadwaita-1-dev libfontconfig-1-dev pkg-config build-essential
rustup default stable
```

Fedora:

```sh
sudo dnf install rustup clang gtk4-devel libadwaita-devel fontconfig-devel pkgconf-pkg-config gcc gcc-c++ make
rustup-init -y
```

GTK 4.22 or newer and libadwaita 1.9 or newer are required.

### Build and install

```sh
cargo build --release
```

The first build compiles the vendored MuPDF sources and takes a few minutes.

Install to `~/.local/bin`:

```sh
cargo install --path . --root ~/.local
```

or copy the binary manually:

```sh
install -Dm755 target/release/pdf-reader ~/.local/bin/pdf-reader
```

Make sure `~/.local/bin` is on your `PATH`.

### Desktop integration

To have the reader show up in your application launcher and as a handler for PDF files,
create `~/.local/share/applications/io.github.ileskaa.PdfReader.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=PDF Reader
Comment=Read PDF documents
Exec=pdf-reader %f
Icon=pdf-reader
Terminal=false
Categories=Office;Viewer;
MimeType=application/pdf;
StartupNotify=true
```

Provide an icon at `~/.local/share/icons/hicolor/scalable/apps/pdf-reader.svg` (any scalable
SVG works; the name must match the `Icon=` key), then refresh the caches:

```sh
update-desktop-database ~/.local/share/applications
gtk4-update-icon-cache -f -t ~/.local/share/icons/hicolor
```

To make it the default PDF handler:

```sh
xdg-mime default io.github.ileskaa.PdfReader.desktop application/pdf
```

## Using it

```sh
pdf-reader                  # reopens the documents that were open last time
pdf-reader a.pdf b.pdf      # opens these documents, each in its own tab
pdf-reader --check file.pdf # headless self-check: render and text-extract every page
```

Documents open as tabs in a single window. Opening a file while the reader is running adds a
tab to it, and opening a document that is already open switches to its tab.

The window follows the desktop's light/dark preference and switches live, without a restart.
Pages themselves are always shown as authored — white paper on a dark canvas in dark mode —
so figures and scans keep their real colours. Each page sits on a soft drop shadow, so its
edge is visible even against the near-white light canvas.

### Keyboard shortcuts

| Shortcut                  | Action                    |
| ------------------------- | ------------------------- |
| `Ctrl+O`                  | Open a document           |
| `Ctrl+F`                  | Find in document          |
| `Enter` / `Shift+Enter`   | Next / previous match     |
| `Escape`                  | Close the find bar        |
| `Ctrl+G` / `Ctrl+Shift+G` | Next / previous match     |
| `Ctrl+A`                  | Select the whole document |
| `Ctrl+C`                  | Copy the selection        |
| `Ctrl++` / `Ctrl+-`       | Zoom in / out             |
| `Ctrl+0`                  | Actual size               |
| `Ctrl+1` / `Ctrl+2`       | Fit width / fit page      |
| `Ctrl+scroll`             | Zoom                      |
| `Ctrl+W`                  | Close the tab             |
| `Ctrl+Tab` / `Ctrl+PgDn`  | Next tab                  |
| `Ctrl+Shift+Tab` / `Ctrl+PgUp` | Previous tab         |
| `Alt+1` … `Alt+9`         | Go to tab 1 … 9           |

Middle-click pastes the current selection into other applications, as in a browser.

## Licence

This project is licensed under AGPL-3.0-or-later; see [`LICENSE`](LICENSE) for the full text.
It links against MuPDF, which Artifex distributes under the AGPL. If you distribute this
reader or run a modified version as a network service, the AGPL requires you to provide the
corresponding source code and comply with its other terms. If your use cannot comply with the
AGPL, Artifex offers a separate commercial licence for MuPDF.
