# Hand testing

## In QEMU (no KVM needed)

```bash
./build.sh amd64
test/make-overlay.sh --out /tmp/overlay.cpio.gz --hostname try --pubkey ~/.ssh/id_ed25519.pub --timeout-min 5
cat dist/agent-amd64.initrd /tmp/overlay.cpio.gz > /tmp/initrd-try

qemu-system-x86_64 -machine pc -cpu max -m 512 -display none -serial mon:stdio -no-reboot \
  -kernel dist/agent-amd64.vmlinuz -initrd /tmp/initrd-try \
  -append 'console=ttyS0,115200n8 loglevel=7' \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2222-:22 -device e1000,netdev=n0
# serial console = root shell, banner "READY try: eth0[...]=10.0.2.15/24 -- ssh root@10.0.2.15"
ssh -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@127.0.0.1 sf-boot status
```

`-device virtio-net-pci` works too; `-no-reboot` turns the watchdog reboot into a QEMU exit. With KVM (`-enable-kvm`) it boots in a few seconds.
To see the watchdog: `--timeout-min 1` and do not log in. To see the beacon: `ssh ... 'nc.traditional -l -u -p 9988'` on the guest (QEMU user networking does not forward guest broadcasts to the host).

Everything the smoke test does is in `test/smoke.sh`; `test/smoke.sh --only "A B"` runs a subset (A: DHCP/SSH/kexec/beacon/password, B: virtio + static + watchdog,
C: claim mode + autorun failure + re-arm, D: autorun success + `sf-boot reboot`, E: console handling), `--dist DIR` points at another build. Artifacts and serial logs of the last run stay in `test/work/` (git-ignored).

## Structural check of any architecture

```bash
test/check-structure.sh arm64      # reads dist/agent-arm64.*; executes nothing of the target
```

## On a real unit (checklist)

1. Have `synofurb-dom backup` done by the DOM component first (the agent never makes backups).
2. Install the agent entry with `synofurb-dom install-agent dist/agent-amd64.vmlinuz initrd-for-this-nas --cmdline '... console=tty0 console=ttyS0,115200n8 panic=30'` — consult the DOM component for flags.
3. Boot it once (one-shot), watch the beacon (`nc -u -l -p 9988` on a PC in the same LAN, or `tcpdump -n udp port 9988`) or the DHCP server's lease list.
4. `ssh root@<ip>`; `sf-boot status` must show `dom_default=ok` and `agent_dom_stub=0`.
5. Do **not** log in the first time if you want to see the safety net: leave it for `AGENT_TIMEOUT_MIN` and confirm DSM comes back.
6. Check `sf-boot net` (every port's driver and addresses) and `dmesg | grep -i firmware` for NICs that need firmware.

## Troubleshooting

* No SSH, but the console shows `READY`: check the key (`sf-boot log dropbear` on the console shell).
* `WARNING: no SSH key and no password configured`: the overlay was not appended, or its config failed to parse (`sf-boot log | head`).
* `dom_default=failed`: `sf-boot log | grep -i dom` shows what `synofurb-dom` said (usually "no Synology DOM found": the storage driver for the DOM is missing, `sf-boot dom status`).
* NIC missing: `dmesg`, `lsmod`, `ls /sys/class/net`; the module may not be in the image (`MANIFEST-<arch>.json` lists the modules; `--modules` adds some).
* Build exceeds the budget: the size report shows where the bytes are; remove modules or try `--suite trixie`.
