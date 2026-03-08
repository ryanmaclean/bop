#!/usr/bin/env nu
# build-qemu-base.nu — build Alpine base image for qemu adapter
#
# Usage:
#   nu scripts/build-qemu-base.nu
#   nu scripts/build-qemu-base.nu --arch x86_64
#
# Produces:
#   ~/.bop/qemu-base.qcow2

def command_exists [name: string]: nothing -> bool {
  ((^which $name | complete).exit_code == 0)
}

def detect_arch [arch_flag: string]: nothing -> string {
  if not ($arch_flag | is-empty) {
    return $arch_flag
  }

  let host_arch = ($nu.os-info.arch | str downcase)
  if ($host_arch == "aarch64") or ($host_arch == "arm64") {
    "aarch64"
  } else {
    "x86_64"
  }
}

def alpine_iso_url [version: string, arch: string]: nothing -> string {
  let parts = ($version | split row ".")
  let major_minor = if ($parts | length) >= 2 {
    ($parts | first 2 | str join ".")
  } else {
    $version
  }
  let iso_name = $"alpine-virt-($version)-($arch).iso"
  $"https://dl-cdn.alpinelinux.org/alpine/v($major_minor)/releases/($arch)/($iso_name)"
}

def find_aarch64_firmware []: nothing -> string {
  let candidates = [
    "/opt/homebrew/share/qemu/edk2-aarch64-code.fd"
    "/usr/local/share/qemu/edk2-aarch64-code.fd"
    "/usr/share/qemu/edk2-aarch64-code.fd"
    "/usr/share/AAVMF/AAVMF_CODE.fd"
  ]

  mut found = ""
  for candidate in $candidates {
    if ($candidate | path exists) {
      $found = $candidate
      break
    }
  }
  $found
}

def machine_profile [arch: string]: nothing -> record<machine: string, cpu: string, firmware: string> {
  if $arch == "aarch64" {
    let firmware = (find_aarch64_firmware)
    if ($firmware | is-empty) {
      print -e "error: missing aarch64 UEFI firmware for QEMU (edk2-aarch64-code.fd)"
      print -e "install qemu firmware and retry, or run: nu scripts/build-qemu-base.nu --arch x86_64"
      exit 1
    }

    if $nu.os-info.name == "macos" {
      {machine: "virt,accel=hvf", cpu: "host", firmware: $firmware}
    } else {
      {machine: "virt", cpu: "cortex-a72", firmware: $firmware}
    }
  } else if $nu.os-info.name == "macos" {
    {machine: "q35,accel=hvf", cpu: "host", firmware: ""}
  } else {
    {machine: "q35", cpu: "", firmware: ""}
  }
}

def write_expect_script [path: string]: nothing -> nothing {
  let script = [
    "#!/usr/bin/expect -f"
    "set timeout [lindex $argv 0]"
    "set qemu_bin [lindex $argv 1]"
    "set iso [lindex $argv 2]"
    "set raw_disk [lindex $argv 3]"
    "set machine [lindex $argv 4]"
    "set cpu [lindex $argv 5]"
    "set firmware [lindex $argv 6]"
    ""
    "set qemu_args [list]"
    "if {$machine ne \"\"} { lappend qemu_args -machine $machine }"
    "if {$cpu ne \"\"} { lappend qemu_args -cpu $cpu }"
    "if {$firmware ne \"\"} { lappend qemu_args -bios $firmware }"
    "lappend qemu_args -m 1024M"
    "lappend qemu_args -drive \"file=$raw_disk,if=virtio,format=raw\""
    "lappend qemu_args -drive \"file=$iso,media=cdrom,readonly=on\""
    "lappend qemu_args -boot order=d"
    "lappend qemu_args -nic user"
    "lappend qemu_args -serial stdio"
    "lappend qemu_args -nographic"
    "lappend qemu_args -no-reboot"
    ""
    "eval spawn $qemu_bin $qemu_args"
    "expect {"
    "  -re {login:} {}"
    "  timeout { puts stderr \"timed out waiting for login prompt\"; exit 1 }"
    "}"
    "send \"root\\r\""
    "expect {"
    "  -re {# $} {}"
    "  timeout { puts stderr \"timed out waiting for root shell\"; exit 1 }"
    "}"
    "set install_cmd {set -eux; setup-interfaces -a || true; setup-apkrepos -1 || true; echo y | setup-disk -m sys /dev/vda; mount /dev/vda3 /mnt || mount /dev/vda2 /mnt || true; chroot /mnt apk add --no-cache cloud-init; chroot /mnt rc-update add cloud-init default; chroot /mnt rc-update add cloud-init-local default || true; sync; poweroff -f}"
    "send -- \"$install_cmd\\r\""
    "expect {"
    "  eof { exit 0 }"
    "  timeout { puts stderr \"timed out waiting for VM shutdown\"; exit 1 }"
    "}"
  ] | str join "\n"

  $script | save --force $path
  ^chmod +x $path
}

def download_iso [url: string, iso_path: string]: nothing -> nothing {
  if ($iso_path | path exists) {
    print $"using existing Alpine ISO: ($iso_path)"
    return
  }

  print $"downloading Alpine ISO: ($url)"
  let result = (^curl -fL --retry 3 -o $iso_path $url | complete)
  if $result.exit_code != 0 {
    print -e $"error: failed to download ISO from ($url)"
    exit 1
  }
}

def run_tests []: nothing -> nothing {
  use std/assert

  assert ((detect_arch "") == "aarch64" or (detect_arch "") == "x86_64") "detect_arch should map host arch"

  let url = (alpine_iso_url "3.21.3" "x86_64")
  assert ($url | str contains "alpine-virt-3.21.3-x86_64.iso") "URL should include Alpine virt ISO name"

  print "PASS: build-qemu-base.nu"
}

def main [
  --arch: string = ""             # Target arch: aarch64 or x86_64 (default: host arch)
  --alpine-version: string = "3.21.3"
  --disk-size: string = "2G"
  --timeout: int = 900            # Install timeout in seconds
  --test                          # Run self-tests
] {
  if $test {
    run_tests
    return
  }

  let resolved_arch = (detect_arch $arch)
  if ($resolved_arch != "aarch64") and ($resolved_arch != "x86_64") {
    print -e $"error: unsupported arch: ($resolved_arch)"
    print -e "supported values: aarch64, x86_64"
    exit 1
  }

  let qemu_bin = $"qemu-system-($resolved_arch)"
  for cmd in ["curl", "qemu-img", "expect", $qemu_bin] {
    if not (command_exists $cmd) {
      print -e $"error: required command not found: ($cmd)"
      exit 1
    }
  }

  let profile = (machine_profile $resolved_arch)

  let bop_home = ("~/.bop" | path expand)
  if not ($bop_home | path exists) {
    mkdir $bop_home
  }

  let iso_name = $"alpine-virt-($alpine_version)-($resolved_arch).iso"
  let iso_path = [$bop_home $iso_name] | path join
  let iso_url = (alpine_iso_url $alpine_version $resolved_arch)
  let base_image = [$bop_home "qemu-base.qcow2"] | path join
  let base_tmp = [$bop_home "qemu-base.qcow2.tmp"] | path join

  download_iso $iso_url $iso_path

  let temp_dir = ((^mktemp -d /tmp/bop-qemu-base.XXXXXX) | str trim)
  let raw_disk = [$temp_dir "alpine-install.raw"] | path join
  let expect_script = [$temp_dir "install.expect"] | path join
  write_expect_script $expect_script

  print $"creating raw install disk: ($raw_disk) size=($disk_size)"
  let create_disk = (^qemu-img create -f raw $raw_disk $disk_size | complete)
  if $create_disk.exit_code != 0 {
    print -e "error: qemu-img create failed"
    ^rm -rf $temp_dir
    exit 1
  }

  print $"booting Alpine installer in QEMU arch=($resolved_arch) timeout=($timeout)s"
  let install = (^expect -f $expect_script ($timeout | into string) $qemu_bin $iso_path $raw_disk $profile.machine $profile.cpu $profile.firmware | complete)
  if $install.exit_code != 0 {
    print -e "error: Alpine install step failed"
    if not ($install.stdout | is-empty) {
      print $install.stdout
    }
    if not ($install.stderr | is-empty) {
      print -e $install.stderr
    }
    print -e $"debug artifacts kept at: ($temp_dir)"
    exit 1
  }

  print $"converting raw disk to qcow2: ($base_image)"
  let convert = (^qemu-img convert -f raw -O qcow2 $raw_disk $base_tmp | complete)
  if $convert.exit_code != 0 {
    print -e "error: qemu-img convert failed"
    print -e $"debug artifacts kept at: ($temp_dir)"
    exit 1
  }

  ^mv -f $base_tmp $base_image
  ^rm -rf $temp_dir

  print $"built QEMU base image: ($base_image)"
}
