%global app_id io.github.djshiye.Clipperino

Name:           clipperino
Version:        1.1.0
Release:        1%{?dist}
Summary:        Clipboard history manager for GNOME

License:        MIT
URL:            https://github.com/djshiye/Clipperino
Source0:        %{url}/releases/download/v%{version}/%{name}-%{version}.tar.gz
# cargo vendor --locked, from the same tag
Source1:        %{url}/releases/download/v%{version}/%{name}-%{version}-vendor.tar.xz

BuildRequires:  cargo-rpm-macros >= 24
BuildRequires:  meson >= 1.0
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.22
BuildRequires:  pkgconfig(libadwaita-1) >= 1.9
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  pkgconfig(gio-2.0)
BuildRequires:  blueprint-compiler
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
BuildRequires:  gettext

Requires:       gtk4%{?_isa} >= 4.22
Requires:       libadwaita%{?_isa} >= 1.9
Requires:       hicolor-icon-theme
# Clipboard monitoring, the global shortcut and autostart all go through portals.
Recommends:     xdg-desktop-portal-gnome
# Renamed from Clipped in 1.1.0.
Obsoletes:      clipped < 1.1.0
Provides:       clipped = %{version}-%{release}

%description
Clipperino keeps a history of the text and images you copy and lets you paste
any of them back into the app you were using. It runs in the background on
GNOME (Wayland) using desktop portals, opens with Super+Shift+V, and includes
a searchable emoji and symbol picker.

%prep
%autosetup -n %{name}-%{version} -a1
%cargo_prep -v vendor

%build
export CARGO_HOME="$PWD/.cargo"
export RUSTFLAGS="%{build_rustflags}"
%meson -Doffline=true
%meson_build

%install
%meson_install

%check
%meson_test

%files
%license LICENSE
%doc README.md docs/FEDORA_REBUILD_PLAN.md
%{_bindir}/%{name}
%{_mandir}/man1/%{name}.1*
%{_datadir}/applications/%{app_id}.desktop
%{_datadir}/dbus-1/services/%{app_id}.service
%{_datadir}/glib-2.0/schemas/%{app_id}.gschema.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg
%{_datadir}/icons/hicolor/symbolic/apps/%{app_id}-symbolic.svg
%{_datadir}/metainfo/%{app_id}.metainfo.xml

%changelog
* Tue Sep 29 2026 djshiye <dreamfantom16@gmail.com> - 1.1.0-1
- Rename from Clipped to Clipperino
- Open the tray menu immediately on click

* Mon Sep 28 2026 djshiye <dreamfantom16@gmail.com> - 1.0.0-1
- Rewrite in Rust with GTK 4 and libadwaita for GNOME on Wayland
