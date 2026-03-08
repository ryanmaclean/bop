use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::util;

const POOL_DIR: &str = ".pool";
const POOL_STATE_FILE: &str = "pool.json";
const POOL_LOCK_REL: &str = ".pool/lock";
const POOL_LOCK_OWNER_FILE: &str = "owner.json";
const POOL_MONITOR_INTERVAL: Duration = Duration::from_secs(5);
const POOL_LEASE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const POOL_BOOT_GRACE_SECONDS: i64 = 15;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VmSlotState {
    Ready,
    Leased { card_id: String },
    Booting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmSlot {
    pub slot_id: usize,
    pub pid: u32,
    pub state: VmSlotState,
    pub qmp_socket: String,
    pub serial_socket: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VmPool {
    #[serde(default)]
    pub size: usize,
    #[serde(default)]
    pub monitor_pid: Option<u32>,
    #[serde(default)]
    pub vms: Vec<VmSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmLease {
    pub slot: usize,
    pub pid: u32,
    pub card_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PoolLockOwner {
    pid: u32,
    started_at: DateTime<Utc>,
}

#[derive(Debug)]
struct PoolLockGuard {
    path: PathBuf,
}

impl Drop for PoolLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn pool_dir(cards_root: &Path) -> PathBuf {
    cards_root.join(POOL_DIR)
}

fn pool_state_path(cards_root: &Path) -> PathBuf {
    pool_dir(cards_root).join(POOL_STATE_FILE)
}

fn pool_slots_dir(cards_root: &Path) -> PathBuf {
    pool_dir(cards_root).join("slots")
}

fn ensure_pool_layout(cards_root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(pool_slots_dir(cards_root))?;
    Ok(())
}

fn acquire_pool_lock(cards_root: &Path) -> anyhow::Result<PoolLockGuard> {
    let lock_dir = cards_root.join(POOL_LOCK_REL);
    if let Some(parent) = lock_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    let owner = PoolLockOwner {
        pid: std::process::id(),
        started_at: Utc::now(),
    };
    let owner_json = serde_json::to_vec_pretty(&owner)?;

    for _ in 0..3 {
        match fs::create_dir(&lock_dir) {
            Ok(()) => {
                let owner_path = lock_dir.join(POOL_LOCK_OWNER_FILE);
                if let Err(err) = fs::write(&owner_path, &owner_json) {
                    let _ = fs::remove_dir_all(&lock_dir);
                    return Err(err.into());
                }
                return Ok(PoolLockGuard { path: lock_dir });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let owner_path = lock_dir.join(POOL_LOCK_OWNER_FILE);
                let stale = fs::read(&owner_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<PoolLockOwner>(&bytes).ok())
                    .map(|existing| !util::pid_is_alive_sync(existing.pid as i32))
                    .unwrap_or(true);

                if stale {
                    let _ = fs::remove_dir_all(&lock_dir);
                    continue;
                }

                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err.into()),
        }
    }

    bail!("pool lock is busy at {}", lock_dir.display())
}

fn load_pool_state(cards_root: &Path) -> anyhow::Result<VmPool> {
    let path = pool_state_path(cards_root);
    if !path.exists() {
        return Ok(VmPool::default());
    }

    let bytes = fs::read(&path).with_context(|| format!("failed reading {}", path.display()))?;
    let mut state: VmPool = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    state.vms.sort_by_key(|s| s.slot_id);
    Ok(state)
}

fn write_pool_state(cards_root: &Path, state: &VmPool) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;

    let path = pool_state_path(cards_root);
    let tmp = pool_dir(cards_root).join(format!("pool.json.tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(state)?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn command_exists(name: &str) -> bool {
    StdCommand::new("which")
        .arg(name)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn detect_qemu_binary() -> String {
    if let Ok(overridden) = std::env::var("BOP_QEMU_BIN") {
        if !overridden.trim().is_empty() {
            return overridden;
        }
    }

    let arch = std::env::consts::ARCH;
    if arch == "aarch64" || arch == "arm64" {
        "qemu-system-aarch64".to_string()
    } else {
        "qemu-system-x86_64".to_string()
    }
}

fn default_base_image() -> PathBuf {
    if let Ok(overridden) = std::env::var("BOP_QEMU_BASE_IMAGE") {
        if !overridden.trim().is_empty() {
            return PathBuf::from(overridden);
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join(".bop").join("qemu-base.qcow2")
}

fn machine_args_for_host() -> Vec<&'static str> {
    let arch = std::env::consts::ARCH;
    if arch == "aarch64" || arch == "arm64" {
        if cfg!(target_os = "macos") {
            vec!["-machine", "virt,accel=hvf", "-cpu", "host"]
        } else {
            vec!["-machine", "virt", "-cpu", "cortex-a72"]
        }
    } else if cfg!(target_os = "macos") {
        vec!["-machine", "q35,accel=hvf", "-cpu", "host"]
    } else {
        vec!["-machine", "q35,accel=tcg"]
    }
}

fn kill_pid(pid: u32, signal: &str) {
    let _ = StdCommand::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status();
}

fn stop_vm_pid(pid: u32) {
    if !util::pid_is_alive_sync(pid as i32) {
        return;
    }

    kill_pid(pid, "-TERM");
    for _ in 0..20 {
        if !util::pid_is_alive_sync(pid as i32) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    if util::pid_is_alive_sync(pid as i32) {
        kill_pid(pid, "-KILL");
    }
}

fn spawn_vm(cards_root: &Path, slot_id: usize) -> anyhow::Result<VmSlot> {
    let qemu_bin = detect_qemu_binary();
    if !command_exists(&qemu_bin) {
        bail!("QEMU binary not found in PATH: {}", qemu_bin);
    }

    let base_image = default_base_image();
    if !base_image.exists() {
        bail!(
            "missing base image: {} (build with `nu scripts/build-qemu-base.nu`)",
            base_image.display()
        );
    }

    let slot_dir = pool_slots_dir(cards_root).join(format!("slot-{}", slot_id));
    fs::create_dir_all(&slot_dir)?;

    let qmp_socket = slot_dir.join("qmp.sock");
    let serial_socket = slot_dir.join("agent.sock");
    let _ = fs::remove_file(&qmp_socket);
    let _ = fs::remove_file(&serial_socket);

    let mut cmd = StdCommand::new(&qemu_bin);
    cmd.args(machine_args_for_host())
        .arg("-snapshot")
        .arg("-m")
        .arg("512M")
        .arg("-drive")
        .arg(format!(
            "file={},if=virtio,format=qcow2",
            base_image.display()
        ))
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp_socket.display()))
        .arg("-chardev")
        .arg(format!(
            "socket,id=bopagent,path={},server=on,wait=off",
            serial_socket.display()
        ))
        .arg("-device")
        .arg("virtio-serial-pci")
        .arg("-device")
        .arg("virtserialport,chardev=bopagent,name=bop.agent")
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg("none")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", qemu_bin))?;

    Ok(VmSlot {
        slot_id,
        pid: child.id(),
        state: VmSlotState::Booting,
        qmp_socket: qmp_socket.to_string_lossy().to_string(),
        serial_socket: serial_socket.to_string_lossy().to_string(),
        started_at: Utc::now(),
    })
}

#[cfg(unix)]
fn ping_monitor_socket(path: &Path) -> bool {
    use std::os::unix::net::UnixStream;

    if !path.exists() {
        return false;
    }

    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));

    let mut buf = [0_u8; 4096];
    let _ = stream.read(&mut buf); // QMP greeting (best-effort)

    if stream
        .write_all(br#"{"execute":"qmp_capabilities"}\n"#)
        .is_err()
    {
        return false;
    }
    let _ = stream.read(&mut buf);

    if stream
        .write_all(br#"{"execute":"query-status"}\n"#)
        .is_err()
    {
        return false;
    }

    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            text.contains("\"return\"") || text.contains("\"running\"")
        }
        _ => false,
    }
}

#[cfg(not(unix))]
fn ping_monitor_socket(_path: &Path) -> bool {
    true
}

fn slot_is_unhealthy(slot: &VmSlot, now: DateTime<Utc>) -> bool {
    if !util::pid_is_alive_sync(slot.pid as i32) {
        return true;
    }

    let qmp_ok = ping_monitor_socket(Path::new(&slot.qmp_socket));
    if qmp_ok {
        return false;
    }

    now.signed_duration_since(slot.started_at).num_seconds() > POOL_BOOT_GRACE_SECONDS
}

fn next_slot_id(state: &VmPool) -> usize {
    state.vms.iter().map(|s| s.slot_id).max().unwrap_or(0) + 1
}

fn reconcile_pool(cards_root: &Path, state: &mut VmPool) -> anyhow::Result<()> {
    let now = Utc::now();

    for idx in 0..state.vms.len() {
        let unhealthy = slot_is_unhealthy(&state.vms[idx], now);
        if unhealthy {
            let slot_id = state.vms[idx].slot_id;
            stop_vm_pid(state.vms[idx].pid);
            let replacement = spawn_vm(cards_root, slot_id)
                .with_context(|| format!("failed replacing slot {}", slot_id))?;
            state.vms[idx] = replacement;
            continue;
        }

        if matches!(state.vms[idx].state, VmSlotState::Booting)
            && ping_monitor_socket(Path::new(&state.vms[idx].qmp_socket))
        {
            state.vms[idx].state = VmSlotState::Ready;
        }
    }

    while state.vms.len() < state.size {
        let slot_id = next_slot_id(state);
        let slot = spawn_vm(cards_root, slot_id)
            .with_context(|| format!("failed to start slot {}", slot_id))?;
        state.vms.push(slot);
    }

    while state.vms.len() > state.size {
        let remove_idx = state
            .vms
            .iter()
            .rposition(|slot| !matches!(slot.state, VmSlotState::Leased { .. }));
        let Some(idx) = remove_idx else {
            break;
        };

        let slot = state.vms.remove(idx);
        stop_vm_pid(slot.pid);
    }

    state.vms.sort_by_key(|s| s.slot_id);
    Ok(())
}

fn lease_ready_slot(state: &mut VmPool, card_id: &str) -> Option<VmLease> {
    for slot in &mut state.vms {
        if matches!(slot.state, VmSlotState::Ready) {
            slot.state = VmSlotState::Leased {
                card_id: card_id.to_string(),
            };
            return Some(VmLease {
                slot: slot.slot_id,
                pid: slot.pid,
                card_id: card_id.to_string(),
            });
        }
    }

    None
}

#[cfg(test)]
fn release_slot_to_ready(state: &mut VmPool, slot_id: usize) -> bool {
    if let Some(slot) = state.vms.iter_mut().find(|s| s.slot_id == slot_id) {
        slot.state = VmSlotState::Ready;
        true
    } else {
        false
    }
}

fn monitor_pid_status(pid: Option<u32>) -> String {
    match pid {
        Some(pid) if util::pid_is_alive_sync(pid as i32) => format!("pid {}", pid),
        Some(pid) => format!("stale pid {}", pid),
        None => "off".to_string(),
    }
}

fn ensure_pool_monitor(cards_root: &Path) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;

    let _guard = acquire_pool_lock(cards_root)?;
    let mut state = load_pool_state(cards_root)?;

    if let Some(pid) = state.monitor_pid {
        if util::pid_is_alive_sync(pid as i32) {
            return Ok(());
        }
    }

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let child = StdCommand::new(exe)
        .arg("--cards-dir")
        .arg(cards_root)
        .arg("factory")
        .arg("pool")
        .arg("monitor")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn pool monitor")?;

    state.monitor_pid = Some(child.id());
    write_pool_state(cards_root, &state)?;

    Ok(())
}

pub fn cmd_pool_set_size(cards_root: &Path, size: usize) -> anyhow::Result<()> {
    if size == 0 {
        return cmd_pool_stop(cards_root);
    }

    ensure_pool_layout(cards_root)?;
    {
        let _guard = acquire_pool_lock(cards_root)?;
        let mut state = load_pool_state(cards_root)?;
        state.size = size;
        reconcile_pool(cards_root, &mut state)?;
        write_pool_state(cards_root, &state)?;
    }

    ensure_pool_monitor(cards_root)?;
    cmd_pool_status(cards_root)
}

pub fn cmd_pool_status(cards_root: &Path) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;
    let state = load_pool_state(cards_root)?;

    let ready = state
        .vms
        .iter()
        .filter(|slot| matches!(slot.state, VmSlotState::Ready))
        .count();
    let leased = state
        .vms
        .iter()
        .filter(|slot| matches!(slot.state, VmSlotState::Leased { .. }))
        .count();
    let booting = state
        .vms
        .iter()
        .filter(|slot| matches!(slot.state, VmSlotState::Booting))
        .count();

    println!(
        "pool size={} ready={} leased={} booting={} monitor={}",
        state.size,
        ready,
        leased,
        booting,
        monitor_pid_status(state.monitor_pid)
    );

    if state.vms.is_empty() {
        println!("  (no slots)");
        return Ok(());
    }

    for slot in &state.vms {
        let state_label = match &slot.state {
            VmSlotState::Ready => "ready".to_string(),
            VmSlotState::Booting => "booting".to_string(),
            VmSlotState::Leased { card_id } => format!("leased({})", card_id),
        };

        println!(
            "  slot={} pid={} state={} qmp={}",
            slot.slot_id, slot.pid, state_label, slot.qmp_socket
        );
    }

    Ok(())
}

pub fn cmd_pool_stop(cards_root: &Path) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;

    let monitor_pid = {
        let _guard = acquire_pool_lock(cards_root)?;
        let mut state = load_pool_state(cards_root)?;

        for slot in &state.vms {
            stop_vm_pid(slot.pid);
        }

        let monitor_pid = state.monitor_pid;
        state.size = 0;
        state.monitor_pid = None;
        state.vms.clear();
        write_pool_state(cards_root, &state)?;
        monitor_pid
    };

    if let Some(pid) = monitor_pid {
        if pid != std::process::id() && util::pid_is_alive_sync(pid as i32) {
            kill_pid(pid, "-TERM");
        }
    }

    println!("■ pool stopped");
    Ok(())
}

pub fn cmd_pool_monitor(cards_root: &Path) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;
    let self_pid = std::process::id();

    loop {
        let mut should_exit = false;

        {
            let _guard = acquire_pool_lock(cards_root)?;
            let mut state = load_pool_state(cards_root)?;

            if let Some(owner_pid) = state.monitor_pid {
                if owner_pid != self_pid && util::pid_is_alive_sync(owner_pid as i32) {
                    return Ok(());
                }
            }

            state.monitor_pid = Some(self_pid);
            reconcile_pool(cards_root, &mut state)?;

            if state.size == 0 && state.vms.is_empty() {
                state.monitor_pid = None;
                should_exit = true;
            }

            write_pool_state(cards_root, &state)?;
        }

        if should_exit {
            return Ok(());
        }

        thread::sleep(POOL_MONITOR_INTERVAL);
    }
}

pub fn cmd_pool_lease(cards_root: &Path, card_id: &str, timeout_s: u64) -> anyhow::Result<()> {
    if card_id.trim().is_empty() {
        bail!("card_id cannot be empty");
    }

    ensure_pool_layout(cards_root)?;
    ensure_pool_monitor(cards_root)?;

    let timeout = timeout_s.max(1);
    let deadline = Instant::now() + Duration::from_secs(timeout);

    loop {
        {
            let _guard = acquire_pool_lock(cards_root)?;
            let mut state = load_pool_state(cards_root)?;

            if state.size == 0 {
                bail!("pool is not running; start with `bop factory pool --size <N>`");
            }

            reconcile_pool(cards_root, &mut state)?;

            if let Some(lease) = lease_ready_slot(&mut state, card_id) {
                write_pool_state(cards_root, &state)?;
                println!("{}", serde_json::to_string(&lease)?);
                return Ok(());
            }

            write_pool_state(cards_root, &state)?;
        }

        if Instant::now() >= deadline {
            bail!("timed out waiting for a ready VM slot after {}s", timeout);
        }

        thread::sleep(POOL_LEASE_POLL_INTERVAL);
    }
}

pub fn cmd_pool_release(
    cards_root: &Path,
    slot: usize,
    card_id: Option<&str>,
    exit_code: i32,
) -> anyhow::Result<()> {
    ensure_pool_layout(cards_root)?;

    {
        let _guard = acquire_pool_lock(cards_root)?;
        let mut state = load_pool_state(cards_root)?;
        let idx = state
            .vms
            .iter()
            .position(|vm| vm.slot_id == slot)
            .with_context(|| format!("slot {} not found", slot))?;

        if let (Some(expected), VmSlotState::Leased { card_id: leased }) =
            (card_id, &state.vms[idx].state)
        {
            if leased != expected {
                eprintln!(
                    "[pool] release mismatch: slot {} leased by '{}' but release requested for '{}'",
                    slot, leased, expected
                );
            }
        }

        stop_vm_pid(state.vms[idx].pid);

        let replacement =
            spawn_vm(cards_root, slot).with_context(|| format!("failed to reset slot {}", slot))?;
        state.vms[idx] = replacement;
        write_pool_state(cards_root, &state)?;
    }

    println!("released slot {} (exit_code={})", slot, exit_code);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_slot(slot_id: usize, state: VmSlotState) -> VmSlot {
        VmSlot {
            slot_id,
            pid: 1000 + slot_id as u32,
            state,
            qmp_socket: format!("/tmp/slot-{slot_id}.qmp"),
            serial_socket: format!("/tmp/slot-{slot_id}.serial"),
            started_at: Utc::now(),
        }
    }

    #[test]
    fn lease_ready_slot_marks_slot_as_leased() {
        let mut state = VmPool {
            size: 2,
            monitor_pid: None,
            vms: vec![
                sample_slot(1, VmSlotState::Ready),
                sample_slot(2, VmSlotState::Ready),
            ],
        };

        let lease = lease_ready_slot(&mut state, "card-a").expect("expected a lease");
        assert_eq!(lease.slot, 1);
        assert_eq!(lease.card_id, "card-a");
        assert!(matches!(
            state.vms[0].state,
            VmSlotState::Leased { ref card_id } if card_id == "card-a"
        ));
    }

    #[test]
    fn lease_ready_slot_skips_non_ready_slots() {
        let mut state = VmPool {
            size: 3,
            monitor_pid: None,
            vms: vec![
                sample_slot(1, VmSlotState::Booting),
                sample_slot(
                    2,
                    VmSlotState::Leased {
                        card_id: "card-b".to_string(),
                    },
                ),
                sample_slot(3, VmSlotState::Ready),
            ],
        };

        let lease = lease_ready_slot(&mut state, "card-c").expect("expected a lease");
        assert_eq!(lease.slot, 3);
        assert!(matches!(
            state.vms[2].state,
            VmSlotState::Leased { ref card_id } if card_id == "card-c"
        ));
    }

    #[test]
    fn lease_ready_slot_returns_none_when_no_ready_slots() {
        let mut state = VmPool {
            size: 2,
            monitor_pid: None,
            vms: vec![
                sample_slot(1, VmSlotState::Booting),
                sample_slot(
                    2,
                    VmSlotState::Leased {
                        card_id: "card-x".to_string(),
                    },
                ),
            ],
        };

        assert!(lease_ready_slot(&mut state, "card-y").is_none());
    }

    #[test]
    fn release_slot_to_ready_changes_state() {
        let mut state = VmPool {
            size: 1,
            monitor_pid: None,
            vms: vec![sample_slot(
                9,
                VmSlotState::Leased {
                    card_id: "card-z".to_string(),
                },
            )],
        };

        assert!(release_slot_to_ready(&mut state, 9));
        assert!(matches!(state.vms[0].state, VmSlotState::Ready));
    }

    #[test]
    fn release_slot_to_ready_returns_false_for_unknown_slot() {
        let mut state = VmPool {
            size: 1,
            monitor_pid: None,
            vms: vec![sample_slot(1, VmSlotState::Ready)],
        };

        assert!(!release_slot_to_ready(&mut state, 999));
    }

    #[test]
    fn pool_state_roundtrip_persists_json() {
        let td = tempdir().unwrap();
        let cards_root = td.path().join(".cards");
        fs::create_dir_all(&cards_root).unwrap();

        let original = VmPool {
            size: 2,
            monitor_pid: Some(12345),
            vms: vec![sample_slot(1, VmSlotState::Ready)],
        };

        write_pool_state(&cards_root, &original).unwrap();
        let loaded = load_pool_state(&cards_root).unwrap();

        assert_eq!(loaded.size, 2);
        assert_eq!(loaded.monitor_pid, Some(12345));
        assert_eq!(loaded.vms.len(), 1);
        assert_eq!(loaded.vms[0].slot_id, 1);
    }
}
