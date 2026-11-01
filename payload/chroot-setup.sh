#!/bin/sh
# synofurb chroot configuration. Runs INSIDE the freshly debootstrapped system (chroot /target).
# Input: /opt/synofurb-install/env.sh (written by the app, plus runtime values from stage 2).
# Every step is logged to stdout/stderr, which stage 2 captures into the install log.
set -eu
export DEBIAN_FRONTEND=noninteractive LC_ALL=C PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
P=/opt/synofurb-install
# shellcheck disable=SC1091
. "$P/env.sh"
say() { echo "[chroot] $*"; }
apt_install() { say "apt-get install $*"; apt-get install -y --no-install-recommends "$@"; }

say "hostname, time zone, locale"
echo "$NAS_HOSTNAME" > /etc/hostname
cat > /etc/hosts <<EOF
127.0.0.1 localhost
127.0.1.1 $NAS_HOSTNAME
::1 localhost ip6-localhost ip6-loopback
ff02::1 ip6-allnodes
ff02::2 ip6-allrouters
EOF
echo "$TIMEZONE" > /etc/timezone

say "apt sources (deb822) for $SUITE"
KR=""
for k in /usr/share/keyrings/debian-archive-keyring.pgp /usr/share/keyrings/debian-archive-keyring.gpg; do
  [ -f "$k" ] && { KR=$k; break; }
done
[ -n "$KR" ] || { say "no Debian archive keyring in the target; refusing to continue with unsigned packages"; exit 1; }
rm -f /etc/apt/sources.list
cat > /etc/apt/sources.list.d/debian.sources <<EOF
Types: deb
URIs: http://$MIRROR_HOST$MIRROR_DIR
Suites: $SUITE $SUITE-updates
Components: main contrib non-free-firmware
Signed-By: $KR

Types: deb
URIs: http://$MIRROR_HOST/debian-security
Suites: $SUITE-security
Components: main contrib non-free-firmware
Signed-By: $KR
EOF
apt-get update
apt_install debian-archive-keyring tzdata
ln -sf "/usr/share/zoneinfo/$TIMEZONE" /etc/localtime

# --- debconf answers so package scripts never prompt or touch the wrong disk -----------------
{
  echo "locales locales/default_environment_locale select $LOCALE"
  echo "locales locales/locales_to_be_generated multiselect $LOCALE UTF-8"
  echo "keyboard-configuration keyboard-configuration/layoutcode string $KEYMAP"
  echo "keyboard-configuration keyboard-configuration/xkb-keymap select $KEYMAP"
  echo "console-setup console-setup/charmap47 select UTF-8"
  echo "tzdata tzdata/Areas select Etc"
  echo "grub2 grub2/force_efi_extra_removable boolean true"
  echo "grub2 grub2/update_nvram boolean false"
  echo "grub-efi-amd64 grub2/force_efi_extra_removable boolean true"
  echo "grub-efi-amd64 grub2/update_nvram boolean false"
  echo "grub-efi-arm64 grub2/force_efi_extra_removable boolean true"
  echo "grub-efi-arm64 grub2/update_nvram boolean false"
  echo "grub-efi-arm grub2/force_efi_extra_removable boolean true"
  echo "grub-efi-arm grub2/update_nvram boolean false"
  echo "grub-pc grub-pc/install_devices_empty boolean true"
  echo "mdadm mdadm/autostart boolean true"
  echo "mdadm mdadm/start_daemon boolean true"
  echo "mdadm mdadm/initrdstart string all"
  echo "mdadm mdadm/mail_to string root"
} | debconf-set-selections 2>/dev/null || true
apt_install debconf locales
sed -i "s/^# *\($LOCALE UTF-8\)/\1/" /etc/locale.gen
locale-gen
update-locale "LANG=$LOCALE"

# --- package list -------------------------------------------------------------------------------
case "$ARCH" in
  amd64) KERNEL=linux-image-amd64 ;;
  arm64) KERNEL=linux-image-arm64 ;;
  armhf) KERNEL=linux-image-armmp ;;
  *) say "unsupported architecture $ARCH"; exit 1 ;;
esac
BOOTPKG=""
case "$FIRMWARE" in
  Bios) BOOTPKG="grub-pc" ;;
  Uefi)
    case "$ARCH" in
      amd64) BOOTPKG="grub-efi-amd64 efibootmgr" ;;
      arm64) BOOTPKG="grub-efi-arm64 efibootmgr" ;;
      armhf) BOOTPKG="grub-efi-arm efibootmgr" ;;
    esac ;;
  UBoot) BOOTPKG="u-boot-tools" ;;
esac
EXTRA=""
[ "$ARCH" = armhf ] && EXTRA="haveged"      # low-entropy ARM boards can hang sshd at boot (Debian wiki)

say "installing kernel, storage stack, filesystems, tools"
# shellcheck disable=SC2086
apt_install $KERNEL initramfs-tools $BOOTPKG \
  mdadm lvm2 xfsprogs btrfs-progs e2fsprogs dosfstools exfatprogs ntfs-3g parted gdisk \
  smartmontools nvme-cli hdparm \
  sudo openssh-server openssh-client \
  systemd-resolved systemd-timesyncd avahi-daemon libnss-mdns \
  python3 openssl ca-certificates curl wget \
  console-setup kbd gpm usbutils pciutils ethtool iproute2 iputils-ping \
  less nano htop rsync bash-completion unattended-upgrades \
  firmware-linux-free firmware-misc-nonfree firmware-realtek \
  $EXTRA $EXTRA_PACKAGES

# --- storage config ---------------------------------------------------------------------------------
say "initramfs: make sure USB keyboards, RAID and LVM are available early"
{
  echo "# Managed by synofurb"
  for m in xhci_pci ehci_pci ohci_pci uhci_hcd usbhid hid_generic atkbd i8042 raid1 raid456 raid10 dm_mod; do echo "$m"; done
} >> /etc/initramfs-tools/modules
sed -i 's/^MODULES=.*/MODULES=most/' /etc/initramfs-tools/initramfs.conf

# --- users --------------------------------------------------------------------------------------------
say "creating user $USERNAME (sudo) and locking root"
useradd -m -s /bin/bash -G sudo,adm,users,dialout "$USERNAME" 2>/dev/null || \
  useradd -m -s /bin/bash -G sudo,adm,users "$USERNAME"
printf '%s:%s\n' "$USERNAME" "$PASSWORD_HASH" | chpasswd -e
passwd -l root
if [ -s "$P/authorized_keys" ]; then
  H=$(getent passwd "$USERNAME" | cut -d: -f6)
  install -d -m 700 -o "$USERNAME" -g "$USERNAME" "$H/.ssh"
  install -m 600 -o "$USERNAME" -g "$USERNAME" "$P/authorized_keys" "$H/.ssh/authorized_keys"
  say "installed $(wc -l < "$P/authorized_keys") SSH public key(s) for $USERNAME"
fi

# --- static configuration files ----------------------------------------------------------------------------
say "installing configuration files"
cp -a "$P/files/." /
chmod 755 /usr/local/sbin/synofurb-firstboot /usr/local/sbin/synofurb-uboot-update 2>/dev/null || true
printf 'synofurb NAS - authorised users only\n' > /etc/issue.net
install -d /etc/synofurb /var/lib/synofurb
cp "$P/plan.json" /var/lib/synofurb/plan.json 2>/dev/null || true
cp "$P/dsm-inspect.txt" /var/lib/synofurb/dsm-inspect.txt 2>/dev/null || true

# --- network: systemd-networkd with DHCPv4 + DHCPv6 + SLAAC ---------------------------------------------------------
say "network: systemd-networkd/resolved, DHCP + SLAAC"
rm -f /etc/network/interfaces
systemctl enable systemd-networkd systemd-resolved systemd-timesyncd avahi-daemon || true
systemctl enable systemd-networkd-wait-online.service 2>/dev/null || true
rm -f /etc/resolv.conf
ln -s /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
sed -i 's/^#\?hosts:.*/hosts:          files mdns4_minimal [NOTFOUND=return] resolve [!UNAVAIL=return] dns/' /etc/nsswitch.conf 2>/dev/null || true

# --- SSH ------------------------------------------------------------------------------------------------------------------
say "ssh: key login preferred, password login allowed, root login disabled"
systemctl enable ssh || true

# --- serial console, VGA console, USB input ---------------------------------------------------------------------------------
say "serial console on $SERIAL_TTY at $SERIAL_SPEED baud + USB serial adapters, VGA console, USB keyboard/mouse"
systemctl enable "serial-getty@$SERIAL_TTY.service" || true
systemctl enable getty@tty1.service || true
systemctl enable gpm || true
cat > /etc/default/console-setup <<EOF
ACTIVE_CONSOLES="/dev/tty[1-6]"
CHARMAP="UTF-8"
CODESET="Lat15"
FONTFACE="Fixed"
FONTSIZE="8x16"
EOF
cat > /etc/default/keyboard <<EOF
XKBMODEL="pc105"
XKBLAYOUT="$KEYMAP"
XKBVARIANT=""
XKBOPTIONS=""
BACKSPACE="guess"
EOF
SERIAL_ARG="console=$SERIAL_TTY,${SERIAL_SPEED}n8"

# --- management plane ---------------------------------------------------------------------------------------------------------
if [ "$ENABLE_MGMT" = 1 ]; then
  say "management plane on https://<ip>:8443"
  install -d -m 755 /opt/synofurb-mgmt
  install -m 644 "$P/mgmt/synofurb_mgmt.py" /opt/synofurb-mgmt/synofurb_mgmt.py
  [ -f "$P/mgmt/README.md" ] && install -m 644 "$P/mgmt/README.md" /opt/synofurb-mgmt/README.md
  install -m 644 "$P/mgmt/synofurb-mgmt.service" /etc/systemd/system/synofurb-mgmt.service
  systemctl enable synofurb-mgmt.service || true
fi
systemctl enable synofurb-firstboot.service || true

# --- bootloader ------------------------------------------------------------------------------------------------------------------
say "bootloader ($FIRMWARE)"
stable_name() { # DEVICE -> a /dev/disk/by-id name if one exists, else the device itself
  _real=$(readlink -f "$1")
  for _l in /dev/disk/by-id/wwn-* /dev/disk/by-id/nvme-eui.* /dev/disk/by-id/ata-* /dev/disk/by-id/scsi-*; do
    [ -e "$_l" ] || continue
    case "$_l" in *-part*) continue ;; esac
    [ "$(readlink -f "$_l")" = "$_real" ] && { echo "$_l"; return; }
  done
  echo "$1"
}
write_grub_cfg() {
  mkdir -p /etc/default/grub.d
  {
    echo 'GRUB_CMDLINE_LINUX="console=tty0 '"$SERIAL_ARG"'"'
    echo 'GRUB_TIMEOUT=5'
    echo 'GRUB_DISABLE_OS_PROBER=true'
    UNIT=${SERIAL_TTY#ttyS}
    case "$UNIT" in
      ''|*[!0-9]*) ;;   # not a PC-style ttyS<N> port: no GRUB serial terminal
      *)
        echo 'GRUB_TERMINAL="console serial"'
        echo "GRUB_SERIAL_COMMAND=\"serial --speed=$SERIAL_SPEED --unit=$UNIT --word=8 --parity=no --stop=1\"" ;;
    esac
  } > /etc/default/grub.d/10-synofurb.cfg
}
case "$FIRMWARE" in
  Bios)
    write_grub_cfg
    DEVS=""
    for d in $SYSTEM_DISKS; do DEVS="$DEVS,$(stable_name "$d")"; done
    echo "grub-pc grub-pc/install_devices multiselect ${DEVS#,}" | debconf-set-selections
    echo "grub-pc grub-pc/install_devices_empty boolean false" | debconf-set-selections
    update-initramfs -u -k all
    update-grub
    for d in $SYSTEM_DISKS; do
      say "grub-install $d"
      grub-install --target=i386-pc "$d"
    done
    ;;
  Uefi)
    write_grub_cfg
    update-initramfs -u -k all
    update-grub
    # --removable puts the loader at EFI/BOOT/BOOT*.EFI on the (mirrored) ESP, which every firmware finds.
    grub-install --target="$GRUB_EFI_TARGET" --efi-directory=/boot/efi --removable --no-nvram
    ;;
  UBoot)
    update-initramfs -u -k all
    cat > /etc/synofurb/uboot.conf <<EOF
SYSTEM_DISKS="$SYSTEM_DISKS"
DTB="${DTB:-}"
ROOT_UUID="$ROOT_UUID"
SERIAL_TTY="$SERIAL_TTY"
SERIAL_SPEED="$SERIAL_SPEED"
KERNEL_MIB=$KERNEL_MIB
INITRD_MIB=$INITRD_MIB
RESERVED_MIB=$RESERVED_MIB
EOF
    if [ "$ARCH" = armhf ]; then
      install -d /etc/kernel/postinst.d /etc/initramfs/post-update.d
      for hook in /etc/kernel/postinst.d/zz-synofurb-uboot /etc/initramfs/post-update.d/zz-synofurb-uboot; do
        printf '#!/bin/sh\nexec /usr/local/sbin/synofurb-uboot-update\n' > "$hook"
        chmod 755 "$hook"
      done
      if [ -z "${DTB:-}" ]; then
        say "WARNING: no DTB configured. Copy your board's .dtb to /etc/synofurb/, set DTB= in /etc/synofurb/uboot.conf and run synofurb-uboot-update."
      else
        /usr/local/sbin/synofurb-uboot-update || say "WARNING: synofurb-uboot-update failed; see messages above"
      fi
    else
      say "WARNING: automatic U-Boot raw-sector boot is only implemented for 32-bit ARM. The system is installed but you must boot it from U-Boot manually (kernel and initrd are in /boot)."
    fi
    ;;
esac

say "cleanup"
apt-get clean
rm -rf /var/lib/apt/lists/* || true
say "chroot configuration finished"
