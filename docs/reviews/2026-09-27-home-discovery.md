# Home discovery dependency removal

The core and CLI used only `dirs::home_dir`; the dirs dependency brought in
MPL-2.0 option-ext. Shared `bop_core::home::directory` now uses Rust's standard
home lookup and rejects an empty returned path before any config/credential
filename is appended. An empty passwd home therefore cannot silently redirect
credentials to the working directory.

The workspace declares Rust 1.87 or newer. Standard API behavior is documented
at https://doc.rust-lang.org/std/env/fn.home_dir.html : Unix checks nonempty HOME
then passwd; Windows checks USERPROFILE then its platform fallback. The Windows
path differs from dirs' Known Folder API and has not been runtime-validated here.
No Windows parity claim is made by the native FreeBSD checks.

The locked graph removes dirs, dirs-sys, option-ext and unused redox_users
without upgrading other packages. Three tests compile the actual production
home module directly on FreeBSD with only the standard library: absent home,
empty passwd home, and preservation of spaces/Unicode in a valid path. Whole
workspace formatting also passes. Full cargo tests/clippy remain unexecuted
because unicode-ident still requires Unicode-3.0 under the stricter project
license instruction. Removing one dependency does not establish full compliance.
