# Candidate measurement host inspection

User supplied root SSH access to10.0.0.50 on2026-09-26 and states it runs on
different physical hardware from mosdns-rust. Credentials are not retained.
Read-only inventory authenticated successfully; no host configuration,
service, SSH authorization, binary or benchmark changes were made.

- Guest hostname: Debian. Local SSH resolution for `test` remains hostname
  test/user tom, not an alias pointing to this address.
- Linux6.19.9-x64v3-xanmod1, x86_64, VMware full virtualization.
- CPU model Intel Pentium Gold8505; one online vCPU, CPU0 only.
- RAM2059714560 bytes; available1703354368 bytes at inspection, no swap.
- ext4 root, approximately6.5GiB available disk.
- mosdns-rust resolves to10.0.0.92, guest hostname mosdns-rust, KVM,
  two online vCPUs; exposed model/family differ. Guest inventory cannot
  independently establish physical-host identity or CPU exclusivity.

This is a useful alternate host candidate, but currently cannot execute the
reviewed separate CPU0/CPU1 plan and fails the2GiB MemAvailable preflight.
Request VM allocation of at least2 vCPUs and4GiB RAM, preferably with reserved
CPU resources/no competing workload. Guest SSH cannot increase hypervisor
vCPU/RAM allocation. After adjustment, reinventory, freeze a fresh prospective
host protocol, obtain selected-reviewer approval and run fresh fixed controls.
M2/M3/M4/V12 verdicts remain unchanged; acceptance stays closed. No weakening
of affinity or memory gates and no measured traffic on this host yet.
