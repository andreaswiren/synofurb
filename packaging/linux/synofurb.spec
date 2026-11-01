# RPM spec for the prebuilt synofurb binary (used by scripts/package-rpm.sh; all %{...} inputs are --define'd there).
%global debug_package %{nil}
%global _build_id_links none
%global __strip /bin/true

Name:           synofurb
Version:        %{pkg_version}
Release:        1
Summary:        Reinstall Synology NAS units with plain Debian 13
License:        MIT
URL:            https://github.com/andreaswiren/synofurb
# runtime libraries that the GPU/windowing stack loads with dlopen() (rpm's automatic ELF dependencies cover the rest)
Requires:       vulkan-loader
Requires:       libxkbcommon
Requires:       libxkbcommon-x11
Requires:       libwayland-client
Requires:       fontconfig
Requires:       alsa-lib
Requires:       ca-certificates

%description
synofurb finds Synology NAS units on your network, inspects them over SSH (read-only), shows exactly what an
installation of plain Debian 13 on LVM + xfs would destroy and keep, boots the Debian installer on the NAS and
streams a detailed log back. Reinstalling repartitions the disks: make a verified backup first.

%prep
# nothing: the binary is prebuilt

%build
# nothing: the binary is prebuilt

%install
install -D -m 0755 %{bin_path}     %{buildroot}%{_bindir}/synofurb
install -D -m 0644 %{icon_path}    %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/synofurb.png
install -D -m 0644 %{desktop_path} %{buildroot}%{_datadir}/applications/synofurb.desktop
install -D -m 0644 %{license_path} %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%{_bindir}/synofurb
%{_datadir}/icons/hicolor/512x512/apps/synofurb.png
%{_datadir}/applications/synofurb.desktop
%license %{_licensedir}/%{name}/LICENSE

%changelog
* Sun Nov 01 2026 Andreas Wiren - %{version}-1
- Automated build from the release workflow.
